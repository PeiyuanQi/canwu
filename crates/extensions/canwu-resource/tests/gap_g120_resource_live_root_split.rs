//! Downstream gap G-120: run budget partitions and their admission epochs
//! persist in live buckets, and the host application revises the run budget
//! of a running root.
//!
//! The tests, in file order:
//!
//! 1. `a_revision_retires_adds_and_changes_totals_all_or_nothing` revises a
//!    detached root: it retires one authority's partition, adds another's,
//!    and changes the totals all or nothing, and marks no report grant dirty.
//!    A stale budget revision, a retired partition the budget does not hold,
//!    an added partition it holds, an `add` out of key order, and a revision
//!    that changes nothing are recorded rejections that leave the budget as
//!    it was; a resent revision returns its stored outcome. A partition
//!    retired and added again comes back with a fresh admission epoch.
//! 2. `a_partition_in_use_is_not_retired` refuses to retire a partition in
//!    use: an acquisition in the lease book; an external participant grant
//!    that is held, prepared, or consumed; and a completion report
//!    reservation that an external participant that completed still waits
//!    on. A released external participant does not hold its partition. A
//!    revision with unchanged totals while both books hold units is
//!    admitted, and the total may fall to what either book holds on its own,
//!    not below.
//! 3. `the_live_root_does_not_grow_with_the_partitions` persists the root
//!    after one lease beside 1 and beside 1,000 partitions: the live root's
//!    encoded size differs by a few digits of its ledger counts, not by the
//!    partitions.
//! 4. `only_the_host_revises_the_budget_of_a_running_root` runs the plugin.
//!    A tracked command cannot carry a revision, and an adapter packet is
//!    refused as `InvalidAuthority`. The host's helper refuses, as
//!    `InvalidPayload` and before queuing, a revision over the change bound
//!    and an `add` out of key order or naming a key twice. A revision another
//!    plugin schedules on the revision ingress settles nothing, with a skip
//!    event. Evidence that is not current is refused when the host queues the
//!    revision; evidence of a kind the plugin does not read, and evidence
//!    superseded between queuing and settlement, are recorded rejections. An
//!    `Acquire` queued in the same boundary after the revision that retires
//!    its authority is refused as one that has no partition, and the new
//!    authority takes a lease. A restored run and an exact replay agree.
//! 5. `only_the_admitting_boundary_writes_a_live_bucket` counts what each
//!    lease step writes: the four steps (and a release before activation) and
//!    the single step, each checked to have applied. Only the boundary that
//!    admits a lease writes a live bucket, the one its partition is in; the
//!    grant, prepare, activate, and release write none. When two partitions
//!    share a bucket, the admission of one leaves the other's entry
//!    byte-identical.
//! 6. `a_partial_reader_attaches_partitions_through_load_run_and_reattach`
//!    reads partitions from a partial root through the host reader
//!    ([`ResourceRootReader::load_run`] and [`ResourceRootReader::reattach`]):
//!    an attached partition answers, one the root does not hold answers none,
//!    and one never attached is an error.
//! 7. `a_tampered_partition_split_is_refused` tampers with the persisted
//!    split: partitions or epochs inline in the live root, a ledger that
//!    miscounts the partitions or the epochs, and an epoch without its
//!    partition are refused.
//! 8. `a_continuation_checkpoint_restores_the_split_root_and_continues_in_lockstep`
//!    cuts a continuation checkpoint after a revision has emptied a live
//!    bucket, with a revision and an `Acquire` of the retired authority
//!    pending: the verified restore has the same hashes and root, and both
//!    runs settle the pending work and a later lease in lockstep.
//! 9. `live_bucket_placement_is_pinned` pins the live bucket of one key of
//!    each kind (account, access grant, definition, unit, custodian, and
//!    partition) at the canonical 32,768 buckets, so a change to the
//!    placement hash or to a key's text, which would move stored records
//!    between buckets without a format change, fails it.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CanwuRun, CommandAttemptOutcome, CommandEnvelope,
    CommandRequest, CommandRequestId, CompactedCanwu, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, ErrorCode, IngressClass,
    IngressPayload, Issuer, KnowledgeHolderRef, PayloadSchema, PersonId, PluginIngressDescriptor,
    PluginIngressRequest, PluginIngressTarget, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateKey, SystemCadence,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "fixture.resource";
const EVIDENCE_NAMESPACE: &str = "fixture.provider";
const PROVIDER: &str = "fixture-provider";
const KEEPER: u64 = 1;
const MERCHANT: u64 = 2;
const NEWCOMER: u64 = 3;

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

/// An initial version of a provider record of kind `kind`.
fn record_version(kind: &str, id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, kind, id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

/// An initial version of a provider evidence record.
fn evidence(id: &str) -> DomainRecordVersionRef {
    record_version("evidence", id)
}

/// The leases of the running fixtures, each locking its own record.
const LOCKED: [&str; 9] = [
    "four",
    "released",
    "single",
    "same-bucket",
    "newcomer",
    "read",
    "tamper",
    "ckpt-keeper",
    "ckpt-newcomer",
];

/// The provider records every running fixture installs: the evidence of
/// budget changes, the envelope's record, each lease's lock, and a record of
/// a kind the resource plugin does not read.
fn fixture_records() -> Vec<DomainRecord> {
    ["works", "lock", "report-source"]
        .into_iter()
        .map(evidence)
        .chain(LOCKED.iter().map(|suffix| lock_version(suffix)))
        .chain([record_version("ledger", "entry")])
        .map(|version| provider_record(&version))
        .collect()
}

fn provider_record(version: &DomainRecordVersionRef) -> DomainRecord {
    DomainRecord {
        reference: version.record.clone(),
        owner: PROVIDER.to_owned(),
        class: DomainRecordClass::Record,
        version: version.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "record": version.record.id }),
        references: Vec::new(),
    }
}

/// The records of authorities that end or begin, and the records leases
/// lock.
struct EvidenceRecord;

impl DomainRecordType for EvidenceRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "evidence";
}

/// A record kind the resource plugin is not configured to read.
struct LedgerRecord;

impl DomainRecordType for LedgerRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "ledger";
}

/// Moves an evidence record named by an admitted `bump` packet to its next
/// version, and schedules the revision an admitted `relay` packet carries on
/// the resource plugin's revision ingress, as a plugin that tried to revise
/// the budget would.
fn bump(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin == PROVIDER && packet_type == "relay" {
            directives.push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: PLUGIN_NAME.to_owned(),
                after: SimDuration::ZERO,
                packet_type: RESOURCE_RUN_BUDGET_REVISION_INGRESS.to_owned(),
                priority: 0,
                payload: payload.clone(),
                affected: Vec::new(),
            });
            continue;
        }
        if plugin != PROVIDER || packet_type != "bump" {
            continue;
        }
        let reference = DomainRecordRef::new(
            EVIDENCE_NAMESPACE,
            "evidence",
            payload["record"].as_str().expect("record"),
        );
        let record = view
            .domain_record(&reference)?
            .expect("evidence record")
            .clone();
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(
                    reference,
                    serde_json::json!({ "record": record.reference.id, "version": record.version + 1 }),
                ),
                expected_version: record.version,
            },
            summary: "The works record moves on".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct ProviderPlugin;

impl SimulationPlugin for ProviderPlugin {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000120"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut evidence = DomainRecordSchema::for_record::<EvidenceRecord>();
        evidence.payload_schema = PayloadSchema::Any;
        let evidence_state = evidence.state_key();
        registrar.register_record_schema(evidence)?;
        let mut ledger = DomainRecordSchema::for_record::<LedgerRecord>();
        ledger.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(ledger)?;
        for (name, description) in [
            ("bump", "Move one evidence record to its next version"),
            (
                "relay",
                "Schedule one run budget revision on the resource plugin",
            ),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class: IngressClass::Information,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        let mut bumping = BoundarySystemContract::new(
            "bump-evidence",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        bumping.reads = vec![StateKey::core_ingress(), evidence_state.clone()];
        bumping.writes = vec![evidence_state];
        bumping.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: RESOURCE_RUN_BUDGET_REVISION_INGRESS.to_owned(),
        }];
        registrar.register_boundary_system(bumping, bump)
    }
}

fn partition(authority: u64) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        authority: holder(authority),
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

fn partition_key(authority: u64) -> CompletionPartitionKeyV1 {
    CompletionPartitionKeyV1::new(holder(authority), NAMESPACE)
}

/// A root with partitions for the keeper and the merchant, one report grant
/// and observation head of the keeper, and the keeper's granary.
fn world() -> ResourceState {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                // Out of key order: the seal sorts them.
                partitions: vec![partition(MERCHANT), partition(KEEPER)],
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
    let resource = ResourceDefinitionRevisionId::new("fixture:resource:grain:v1").expect("id");
    let scope = ResourceScopeId::new("fixture:scope:market").expect("scope");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("fixture:resource:grain").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit.clone(),
            quality: ResourceQualityId::new("fixture:quality:standard").expect("quality"),
            scope: scope.clone(),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    state
        .install_opening_account(ResourceAccount {
            id: account("granary"),
            revision: ResourceRevision::INITIAL,
            custodian: holder(KEEPER),
            resource_revision: resource,
            unit_revision: unit,
            balance: 50,
            capacity: None,
            protected_floor_policy: None,
            closed: false,
            place_scope: None,
        })
        .expect("account");
    let report_grant = ResourceReportGrantId::new("fixture:report-grant:keeper").expect("grant");
    state
        .install_report_grant(ResourceReportGrantV1 {
            id: report_grant.clone(),
            holder: holder(KEEPER),
            scope,
            accounts: BTreeSet::new(),
            demands: BTreeSet::new(),
            include_transfer_details: false,
            confidence_per_mille: 1_000,
            cadence_minutes: 1,
            delay_minutes: 0,
        })
        .expect("report grant");
    let source = evidence("report-source");
    let revision = state.state_revision;
    state
        .record_observation_head(
            ResourceObservationHeadV1 {
                id: ResourceObservationHeadId::new("fixture:observation:keeper")
                    .expect("observation"),
                revision: ResourceRevision::INITIAL,
                provider_plugin: PROVIDER.to_owned(),
                provider_version: "1".to_owned(),
                provider_semantic_hash: "f".repeat(64),
                provider_source: source.clone(),
                holder: holder(KEEPER),
                grant: report_grant,
                provider_state_revision: revision,
                observed_at: SimTime::EPOCH,
                confidence_per_mille: 1_000,
                stock: Vec::new(),
                demands: Vec::new(),
                allocations: Vec::new(),
                fulfillments: Vec::new(),
                transfers: Vec::new(),
                consumptions: Vec::new(),
                losses: Vec::new(),
                source_versions: vec![source],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("observation"),
        )
        .expect("record observation");
    state
}

/// A revision of `state`'s budget at its current revision and totals.
fn revision(
    state: &ResourceState,
    name: &str,
    retire: &[u64],
    add: &[u64],
) -> ResourceReviseRunBudgetRequestV1 {
    ResourceReviseRunBudgetRequestV1 {
        operation_key: key(&format!("fixture:revise:{name}")),
        expected_budget_revision: state.run_budget.revision,
        total_completion_units: state.run_budget.total_completion_units,
        shared_pending_slots: state.run_budget.shared_pending_slots,
        retire: retire.iter().map(|value| partition_key(*value)).collect(),
        add: add.iter().map(|value| partition(*value)).collect(),
        evidence: evidence("works"),
    }
}

fn revise(request: &ResourceReviseRunBudgetRequestV1) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::ReviseRunBudget(request.clone())
}

fn completion(operation: ResourceCompletionOperationV1) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::Completion(operation)
}

/// An applied revision that keeps its evidence and advances the budget.
fn applied(state: &mut ResourceState, request: &ResourceReviseRunBudgetRequestV1) {
    let before = state.run_budget.revision;
    let outcome = state.apply_operation(&revise(request)).expect("revision");
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(outcome.kind, ResourceOperationKind::ReviseRunBudget);
    assert_eq!(outcome.exact_evidence, vec![request.evidence.clone()]);
    assert_eq!(
        outcome.quantity,
        u64::try_from(request.retire.len() + request.add.len()).expect("changes")
    );
    assert_eq!(outcome.result_ref, None);
    assert_eq!(
        state.run_budget.revision,
        before.next().expect("budget revision")
    );
    assert_eq!(
        state.run_budget.clone().seal().expect("reseal"),
        state.run_budget,
        "the budget stays sorted and sealed"
    );
    state.validate().expect("root after a revision");
}

/// A revision that is the recorded rejection `code` and leaves the budget
/// and the epochs as they were.
fn refused(state: &mut ResourceState, request: &ResourceReviseRunBudgetRequestV1, code: &str) {
    let budget = state.run_budget.clone();
    let epochs = state.completion_leases.epochs.clone();
    let outcome = state
        .apply_operation(&revise(request))
        .expect("a refused revision is a recorded rejection");
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{code}");
    assert_eq!(outcome.kind, ResourceOperationKind::ReviseRunBudget);
    assert_eq!(
        outcome.rejection_code.as_deref(),
        Some(code),
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(state.run_budget, budget, "{code}");
    assert_eq!(state.completion_leases.epochs, epochs, "{code}");
    state.validate().expect("root after a refused revision");
}

fn recipe(reports: u16) -> CompletionCapacityRecipeV1 {
    CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 1,
        reports_per_holder: reports,
        holders: reports,
        bytes: 1_024,
    }
}

fn envelope() -> EligibilityEnvelopeV1 {
    EligibilityEnvelopeV1::new(
        vec![evidence("lock")],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope")
}

/// The record the lease `suffix` locks: its own, so that one lease left
/// open does not hold another's target.
fn lock_version(suffix: &str) -> DomainRecordVersionRef {
    evidence(&format!("lock-{suffix}"))
}

fn lock_target(suffix: &str) -> CompletionLockedTargetV1 {
    CompletionLockedTargetV1::ExternalRecord {
        version: lock_version(suffix),
    }
}

fn lease_request(lease_holder: u64, suffix: &str, at: SimTime) -> RequestCompletionLeaseV1 {
    RequestCompletionLeaseV1 {
        id: acquisition(suffix),
        operation_key: key(&format!("fixture:operation:{suffix}")),
        holder: holder(lease_holder),
        operation_namespace: NAMESPACE.to_owned(),
        eligibility_time: at,
        eligibility_envelope: envelope(),
        recipe: recipe(0),
        expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
        policy_class: CompletionPolicyClassV1::Guaranteed,
    }
}

fn grant_request(
    state: &ResourceState,
    suffix: &str,
    current_boundary: u64,
) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
        grant_id: grant(suffix),
        acquisition: acquisition(suffix),
        expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition(suffix)]
            .revision,
        owner_plugin: PLUGIN_NAME.to_owned(),
        target_versions: vec![lock_target(suffix)],
        current_boundary,
    })
}

fn prepare_request(
    state: &ResourceState,
    suffix: &str,
    current_boundary: u64,
) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
        acquisition: acquisition(suffix),
        expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition(suffix)]
            .revision,
        grant: grant(suffix),
        expected_grant_revision: state.completion_leases.grants[&grant(suffix)].revision,
        current_boundary,
        eligibility_envelope_digest: envelope().digest,
    })
}

fn activate_request(
    state: &ResourceState,
    suffix: &str,
    at: SimTime,
    current_boundary: u64,
) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
        acquisition: acquisition(suffix),
        expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition(suffix)]
            .revision,
        grant: grant(suffix),
        expected_grant_revision: state.completion_leases.grants[&grant(suffix)].revision,
        at,
        current_boundary,
        eligibility_envelope_digest: envelope().digest,
    })
}

fn release_request(state: &ResourceState, suffix: &str) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::Release(ReleaseCompletionCapacityV1 {
        acquisition: acquisition(suffix),
        expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition(suffix)]
            .revision,
        grant: grant(suffix),
        expected_grant_revision: state.completion_leases.grants[&grant(suffix)].revision,
        reason: "fixture_release".to_owned(),
    })
}

fn single_step(
    lease_holder: u64,
    suffix: &str,
    at: SimTime,
    boundary: u64,
) -> SingleStepCompletionLeaseV1 {
    SingleStepCompletionLeaseV1 {
        lease: lease_request(lease_holder, suffix, at),
        grant_id: grant(suffix),
        target_versions: vec![lock_target(suffix)],
        current_boundary: boundary,
    }
}

fn settle_applied(state: &mut ResourceState, request: ResourceOperationRequestV1) {
    let outcome = state.apply_operation(&request).expect("operation");
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
}

/// Takes a lease of `lease_holder` and gives it up before activation, so its
/// partition has an admission epoch and no acquisition is left in the book.
fn lease_and_release(state: &mut ResourceState, lease_holder: u64, suffix: &str) {
    settle_applied(
        state,
        completion(ResourceCompletionOperationV1::Acquire(lease_request(
            lease_holder,
            suffix,
            SimTime::EPOCH,
        ))),
    );
    let grant_step = grant_request(state, suffix, 1);
    settle_applied(state, completion(grant_step));
    let release = release_request(state, suffix);
    settle_applied(state, completion(release));
    assert!(
        !state
            .completion_leases
            .acquisitions
            .contains_key(&acquisition(suffix)),
        "a lease released before activation leaves the book"
    );
}

/// An external participant grant of `participant_holder`, with `reports`
/// named report reservations.
fn external_grant(
    participant_holder: u64,
    suffix: &str,
    reports: u16,
) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::GrantExternalParticipant(
        RequestExternalCompletionParticipantGrantV1 {
            coordinator_plugin: PROVIDER.to_owned(),
            coordinator_source: evidence(&format!("coordinator-{suffix}")),
            coordinator_acquisition_revision: ResourceRevision::INITIAL,
            acquisition: acquisition(suffix),
            operation_key: key(&format!("fixture:external:{suffix}")),
            holder: holder(participant_holder),
            operation_namespace: NAMESPACE.to_owned(),
            eligibility_time: SimTime::EPOCH,
            eligibility_envelope_digest: envelope().digest,
            recipe: recipe(reports),
            policy_class: CompletionPolicyClassV1::Guaranteed,
            grant_id: grant(suffix),
            target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                version: evidence(&format!("target-{suffix}")),
            }],
            current_boundary: 1,
        },
    )
}

fn external_prepare(state: &ResourceState, suffix: &str) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::PrepareExternalParticipant(
        PrepareExternalCompletionParticipantGrantV1 {
            coordinator_source: evidence(&format!("coordinator-{suffix}")),
            acquisition: acquisition(suffix),
            expected_grant_revision: state.external_completion_participants.grants
                [&acquisition(suffix)]
                .grant
                .revision,
            current_boundary: 2,
            eligibility_envelope_digest: envelope().digest,
        },
    )
}

fn external_release(state: &ResourceState, suffix: &str) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::ReleaseExternalParticipant(
        ReleaseExternalCompletionParticipantGrantV1 {
            coordinator_source: evidence(&format!("coordinator-{suffix}")),
            acquisition: acquisition(suffix),
            expected_grant_revision: state.external_completion_participants.grants
                [&acquisition(suffix)]
                .grant
                .revision,
            reason: "fixture_release".to_owned(),
        },
    )
}

/// The activation certificate a coordinator issues for the prepared
/// external participant `suffix`.
fn external_certificate(
    state: &ResourceState,
    suffix: &str,
) -> CompletionLeaseActivationCertificateV1 {
    let prepared = state.external_completion_participants.grants[&acquisition(suffix)]
        .grant
        .clone();
    let mut certificate = CompletionLeaseActivationCertificateV1 {
        acquisition: acquisition(suffix),
        acquisition_revision: ResourceRevision::INITIAL,
        operation_key: key(&format!("fixture:external:{suffix}")),
        prepared_grants: vec![(prepared.id.clone(), prepared.revision)],
        locked_target_versions: prepared.target_versions.clone(),
        recipe_digest: prepared.recipe_digest.clone(),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope_digest: envelope().digest,
        activation_boundary: 3,
        semantic_digest: String::new(),
    };
    certificate.semantic_digest = digest_of(
        "canwu.resource.completion-activation-certificate.v1",
        &certificate,
    );
    certificate
}

/// The canonical digest the engine computes: BLAKE3 of the domain, a zero
/// byte, and the JSON encoding.
fn digest_of<T: serde::Serialize>(domain: &str, value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("canonical test encoding");
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&bytes);
    hasher.finalize().to_hex().to_string()
}

#[test]
fn a_revision_retires_adds_and_changes_totals_all_or_nothing() {
    let mut state = world();
    assert_eq!(
        state
            .run_budget
            .partitions
            .iter()
            .map(CompletionCapacityPartitionV1::key)
            .collect::<Vec<_>>(),
        vec![partition_key(KEEPER), partition_key(MERCHANT)],
        "the install sorts the partitions"
    );
    assert_eq!(state.partition_count(), 2);
    assert!(
        state
            .holds_partition(&partition_key(KEEPER))
            .expect("whole")
    );

    // The merchant's authority ends and a newcomer's begins. The revision
    // changes nothing a holder report shows, so it marks no report grant
    // dirty.
    state.report_dirty_grants.clear();
    let swap = revision(&state, "swap", &[MERCHANT], &[NEWCOMER]);
    applied(&mut state, &swap);
    assert!(
        state.report_dirty_grants.is_empty(),
        "a revision marks no report grant dirty"
    );
    assert_eq!(
        state
            .run_budget
            .partitions
            .iter()
            .map(CompletionCapacityPartitionV1::key)
            .collect::<Vec<_>>(),
        vec![partition_key(KEEPER), partition_key(NEWCOMER)]
    );

    // A stale budget revision, a partition the budget does not hold, one it
    // holds, an `add` out of key order or twice, too many changes, and a
    // revision that changes nothing are recorded rejections.
    let mut stale = revision(&state, "stale", &[], &[4]);
    stale.expected_budget_revision = ResourceRevision::INITIAL;
    refused(&mut state, &stale, "version_conflict");
    // A resent revision returns its stored outcome and changes nothing.
    let budget = state.run_budget.clone();
    let resent = state
        .apply_operation(&revise(&swap))
        .expect("a resent revision");
    assert_eq!(resent.status, ResourceOperationStatus::Applied);
    assert_eq!(state.run_budget, budget);
    let gone = revision(&state, "gone", &[MERCHANT], &[]);
    refused(&mut state, &gone, "not_found");
    let held = revision(&state, "held", &[], &[KEEPER]);
    refused(&mut state, &held, "idempotency_conflict");
    let unsorted = revision(&state, "unsorted", &[], &[5, 4]);
    refused(&mut state, &unsorted, "invalid_definition");
    let twice = revision(&state, "twice", &[], &[4, 4]);
    refused(&mut state, &twice, "invalid_definition");
    let nothing = revision(&state, "nothing", &[], &[]);
    refused(&mut state, &nothing, "invalid_definition");
    let mut too_many = revision(&state, "too-many", &[], &[]);
    too_many.add = (100..=u64::try_from(100 + MAX_RUN_BUDGET_REVISION_CHANGES).expect("count"))
        .map(partition)
        .collect();
    refused(&mut state, &too_many, "limit_exceeded");
    let mut invalid = revision(&state, "invalid", &[], &[4]);
    invalid.add[0].reserved_pending_slots = 0;
    refused(&mut state, &invalid, "limit_exceeded");

    // The guarantees and reserved slots after the change must fit the
    // totals; a revision may raise the totals to make room. Each attempt
    // is a new operation: a refused key keeps its recorded outcome.
    let crowded = revision(&state, "crowded", &[], &[4, 5, 6, 7]);
    refused(&mut state, &crowded, "limit_exceeded");
    let mut roomy = revision(&state, "roomy", &[], &[4, 5, 6, 7]);
    roomy.total_completion_units = 4_000_000;
    applied(&mut state, &roomy);
    assert_eq!(state.run_budget.total_completion_units, 4_000_000);
    assert_eq!(state.partition_count(), 6);
    let mut slots = revision(&state, "slots", &[], &[]);
    slots.shared_pending_slots =
        u16::try_from(MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL).expect("slots");
    refused(&mut state, &slots, "limit_exceeded");
    let mut fewer_slots = revision(&state, "fewer-slots", &[], &[]);
    fewer_slots.shared_pending_slots = 16;
    applied(&mut state, &fewer_slots);

    // A partition retired and added again in one revision comes back with a
    // fresh epoch: its backward-time guard restarts.
    lease_and_release(&mut state, KEEPER, "epoch");
    assert!(
        state
            .admission_epoch(&partition_key(KEEPER))
            .expect("whole")
            .is_some()
    );
    assert!(
        !state.report_dirty_grants.is_empty(),
        "an applied lease step does mark the report grants dirty"
    );
    let readd = revision(&state, "readd", &[KEEPER], &[KEEPER]);
    applied(&mut state, &readd);
    assert!(
        state
            .admission_epoch(&partition_key(KEEPER))
            .expect("whole")
            .is_none()
    );
    assert_eq!(
        state.partition(&partition_key(KEEPER)).expect("whole"),
        Some(&partition(KEEPER))
    );

    // A retired authority's lease is refused as one with no partition.
    let after = revision(&state, "retire-keeper", &[KEEPER], &[]);
    applied(&mut state, &after);
    let outcome = state
        .apply_operation(&completion(ResourceCompletionOperationV1::Acquire(
            lease_request(KEEPER, "retired", minute(5)),
        )))
        .expect("a recorded rejection");
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_authority"));
    // The newcomer leases.
    settle_applied(
        &mut state,
        completion(ResourceCompletionOperationV1::SingleStepLease(single_step(
            NEWCOMER,
            "newcomer",
            minute(5),
            4,
        ))),
    );
}

#[test]
fn a_partition_in_use_is_not_retired() {
    // An acquisition in the lease book.
    let mut state = world();
    settle_applied(
        &mut state,
        completion(ResourceCompletionOperationV1::Acquire(lease_request(
            MERCHANT,
            "requested",
            SimTime::EPOCH,
        ))),
    );
    let retire = revision(&state, "busy-lease", &[MERCHANT], &[]);
    refused(&mut state, &retire, "invalid_lifecycle");
    // The keeper's partition is not in use and leaves.
    let keeper = revision(&state, "idle-keeper", &[KEEPER], &[]);
    applied(&mut state, &keeper);

    // An external participant grant that is held, then prepared; a released
    // one does not hold its partition.
    let mut state = world();
    let held = external_grant(MERCHANT, "held", 0);
    settle_applied(&mut state, completion(held));
    let retire = revision(&state, "busy-held", &[MERCHANT], &[]);
    refused(&mut state, &retire, "invalid_lifecycle");
    let prepare = external_prepare(&state, "held");
    settle_applied(&mut state, completion(prepare));
    let retire = revision(&state, "busy-prepared", &[MERCHANT], &[]);
    refused(&mut state, &retire, "invalid_lifecycle");
    let release = external_release(&state, "held");
    settle_applied(&mut state, completion(release));
    let retire = revision(&state, "released", &[MERCHANT], &[]);
    applied(&mut state, &retire);

    // A consumed external participant; then, completed, its named report
    // reservation waits for the keeper's report and still holds the
    // partition.
    let mut state = world();
    let granted = external_grant(KEEPER, "reported", 1);
    settle_applied(&mut state, completion(granted));
    assert!(
        state
            .completion_report_reservations
            .contains_key(&acquisition("reported"))
    );
    let prepare = external_prepare(&state, "reported");
    settle_applied(&mut state, completion(prepare));
    let certificate = external_certificate(&state, "reported");
    settle_applied(
        &mut state,
        completion(ResourceCompletionOperationV1::ConsumeExternalParticipant(
            ConsumeExternalCompletionParticipantGrantV1 {
                coordinator_source: evidence("coordinator-reported"),
                certificate,
                at: SimTime::EPOCH,
            },
        )),
    );
    let retire = revision(&state, "busy-consumed", &[KEEPER], &[]);
    refused(&mut state, &retire, "invalid_lifecycle");
    settle_applied(
        &mut state,
        completion(ResourceCompletionOperationV1::CompleteExternalParticipant(
            CompleteExternalCompletionParticipantGrantV1 {
                acquisition: acquisition("reported"),
                operation_key: key("fixture:external:reported"),
            },
        )),
    );
    assert_eq!(
        state
            .external_completion_participants
            .participant(&acquisition("reported"))
            .expect("participant")
            .grant
            .state,
        CompletionGrantStateV1::Completed
    );
    assert!(
        state
            .completion_report_reservations
            .contains_key(&acquisition("reported")),
        "the completed participant still waits for its report"
    );
    let retire = revision(&state, "busy-report", &[KEEPER], &[]);
    refused(&mut state, &retire, "invalid_lifecycle");

    // While both books hold units, a revision with unchanged totals is
    // admitted, and the total may fall to what either book holds on its own
    // (below their sum) but not below. The partitions guarantee nothing here,
    // so only the books bound the total.
    let mut state = world();
    let mut budget = state.run_budget.clone();
    for partition in &mut budget.partitions {
        partition.guaranteed_units = 0;
    }
    state
        .install_run_budget(budget.seal().expect("budget"))
        .expect("reinstall before any lease");
    // With no guarantee, both leases draw on the shared pool.
    let mut units_lease = lease_request(KEEPER, "units", SimTime::EPOCH);
    units_lease.policy_class = CompletionPolicyClassV1::SharedBurst;
    settle_applied(
        &mut state,
        completion(ResourceCompletionOperationV1::Acquire(units_lease)),
    );
    let grant_step = grant_request(&state, "units", 1);
    settle_applied(&mut state, completion(grant_step));
    let ResourceCompletionOperationV1::GrantExternalParticipant(mut external) =
        external_grant(MERCHANT, "units-external", 0)
    else {
        unreachable!("an external participant grant")
    };
    external.policy_class = CompletionPolicyClassV1::SharedBurst;
    settle_applied(
        &mut state,
        completion(ResourceCompletionOperationV1::GrantExternalParticipant(
            external,
        )),
    );
    let local_units = state.completion_leases.reserved_units;
    let external_units = state.external_completion_participants.reserved_units;
    assert!(local_units > 0 && external_units > 0);
    let mut unchanged = revision(&state, "unchanged-totals", &[], &[NEWCOMER]);
    unchanged.add[0].guaranteed_units = 0;
    applied(&mut state, &unchanged);
    let floor = local_units.max(external_units);
    assert!(floor < local_units + external_units);
    let mut below_books = revision(&state, "below-books", &[], &[]);
    below_books.total_completion_units = floor - 1;
    refused(&mut state, &below_books, "limit_exceeded");
    let mut at_books = revision(&state, "at-books", &[], &[]);
    at_books.total_completion_units = floor;
    applied(&mut state, &at_books);
}

/// A partition of `authority` that reserves one pending slot and guarantees
/// nothing, so that a thousand fit the global pool.
fn small_partition(authority: u64) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        guaranteed_units: 0,
        reserved_pending_slots: 1,
        ..partition(authority)
    }
}

/// The encoded bytes of the live root `state` persists as.
fn live_root_bytes(state: &ResourceState) -> usize {
    let records = state.clone().into_records().expect("the root persists");
    let live = records
        .iter()
        .find(|record| record.reference.id == "resource:runtime:v1")
        .expect("the live root");
    serde_json::to_vec(&live.payload)
        .expect("the live root encodes")
        .len()
}

#[test]
fn the_live_root_does_not_grow_with_the_partitions() {
    // One partition: the keeper's.
    let mut few = world();
    let retire_merchant = revision(&few, "one", &[MERCHANT], &[]);
    applied(&mut few, &retire_merchant);
    assert_eq!(few.partition_count(), 1);

    // A thousand: the keeper's, the merchant's, and 998 more, added 256 at a
    // time.
    let mut many = world();
    let authorities: Vec<u64> = (100..1_098).collect();
    for (index, chunk) in authorities
        .chunks(MAX_RUN_BUDGET_REVISION_CHANGES)
        .enumerate()
    {
        let mut request = revision(&many, &format!("many:{index}"), &[], &[]);
        request.add = chunk.iter().map(|value| small_partition(*value)).collect();
        request.add.sort_by_key(CompletionCapacityPartitionV1::key);
        applied(&mut many, &request);
    }
    assert_eq!(many.partition_count(), 1_000);

    // One lease of the keeper on each, then the persisted live root.
    lease_and_release(&mut few, KEEPER, "size");
    lease_and_release(&mut many, KEEPER, "size");
    let (few_bytes, many_bytes) = (live_root_bytes(&few), live_root_bytes(&many));
    assert!(
        few_bytes.abs_diff(many_bytes) <= 64,
        "live root of {few_bytes} bytes beside 1 partition, {many_bytes} beside 1,000"
    );
    let inline = serde_json::to_vec(&many.run_budget.partitions)
        .expect("the partitions encode")
        .len();
    assert!(
        inline > 64 * 1_000,
        "the partitions alone are {inline} bytes, none of them in the live root"
    );
}

fn plugin() -> ResourcePlugin {
    ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")])
}

fn runtime(seed: u64, resource_plugin: &ResourcePlugin, root: ResourceState) -> Canwu {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        [KEEPER, MERCHANT, NEWCOMER]
            .into_iter()
            .map(|value| EntityRef::Person(PersonId::new(value)))
            .collect(),
    )
    .with_domain_records(
        fixture_records()
            .into_iter()
            .chain(root.into_records().expect("resource records"))
            .collect(),
    );
    Canwu::new_with_plugins(seed, scenario, &[resource_plugin, &ProviderPlugin]).expect("runtime")
}

fn state<H: CanwuRun + ?Sized>(canwu: &H) -> ResourceState {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
}

/// The earliest time any queued ingress is due.
fn earliest_due(canwu: &Canwu) -> Option<SimTime> {
    canwu.pending_ingress().map(|record| record.due_at).min()
}

/// Settles every queued ingress due before `at`, each at its own instant.
/// The kernel settles no boundary past queued ingress, and the resource
/// plugin queues ingress of its own: a report wake a cadence after each
/// report, and a completion expiry tick at the same instant while a grant
/// waits. A fixture that moves on to a later instant settles those first.
fn catch_up(canwu: &mut Canwu, at: SimTime) {
    let mut boundaries = 0_u32;
    while let Some(due) = earliest_due(canwu).filter(|due| *due < at) {
        boundaries += 1;
        assert!(boundaries <= 64, "the fixture's own ingress settles");
        canwu
            .settle_boundary(BoundaryRequest::at(due))
            .expect("a resource boundary never fails in this fixture");
    }
}

/// Settles the boundary at `at`, after any ingress due before it.
fn settle(canwu: &mut Canwu, at: SimTime) {
    catch_up(canwu, at);
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

/// Submits one tracked command from `actor` and settles it when admitted.
fn command(
    canwu: &mut Canwu,
    request_id: u64,
    actor: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
) -> CommandAttemptOutcome {
    canwu
        .enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
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
    settle(canwu, at);
    let outcome = canwu
        .command_attempts()
        .last()
        .expect("command attempt")
        .outcome
        .clone();
    if matches!(outcome, CommandAttemptOutcome::Accepted { .. }) {
        settle(canwu, at);
    }
    outcome
}

#[test]
fn only_the_host_revises_the_budget_of_a_running_root() {
    let resource_plugin = plugin();
    let mut canwu = runtime(120, &resource_plugin, world());
    let at = minute(1);

    // A tracked command cannot carry a revision, and records nothing.
    let by_keeper = revision(&state(&canwu), "by-keeper", &[MERCHANT], &[]);
    let attempt = command(&mut canwu, 1, KEEPER, at, revise(&by_keeper));
    assert!(
        matches!(attempt, CommandAttemptOutcome::Rejected { .. }),
        "{attempt:?}"
    );
    assert!(
        !state(&canwu)
            .outcomes
            .contains_key(&by_keeper.operation_key)
    );

    // Nor can an adapter packet.
    let error = resource_adapter_ingress(
        at,
        &ResourceAdapterOperationV1 {
            provider_plugin: PROVIDER.to_owned(),
            provider_source: evidence("works"),
            request: revise(&by_keeper),
        },
    )
    .expect_err("an adapter packet carries no revision");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);

    // The host's own helper refuses a revision over the change bound, and an
    // `add` out of key order or naming a key twice, before it is queued.
    let mut oversized = revision(&state(&canwu), "oversized", &[], &[]);
    let bound = u64::try_from(MAX_RUN_BUDGET_REVISION_CHANGES).expect("bound");
    oversized.add = (0..=bound)
        .map(|value| small_partition(1_000 + value))
        .collect();
    let error = enqueue_resource_run_budget_revision(&mut canwu, at, &oversized)
        .expect_err("more keys than a revision may change");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    let unsorted = revision(&state(&canwu), "unsorted", &[], &[5, 4]);
    let error = enqueue_resource_run_budget_revision(&mut canwu, at, &unsorted)
        .expect_err("an add out of key order");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    let repeated = revision(&state(&canwu), "repeated", &[], &[4, 4]);
    let error = enqueue_resource_run_budget_revision(&mut canwu, at, &repeated)
        .expect_err("an add naming a key twice");
    assert_eq!(error.code, ErrorCode::InvalidPayload);

    // Evidence that is not its record's current version is refused when the
    // host queues the revision.
    let mut stale = revision(&state(&canwu), "stale", &[MERCHANT], &[]);
    stale.evidence.version = 2;
    let error =
        enqueue_resource_run_budget_revision(&mut canwu, at, &stale).expect_err("stale evidence");
    assert_eq!(error.code, ErrorCode::EvidenceUnavailable);

    // A revision another plugin schedules on the revision ingress settles
    // nothing and records no outcome: its boundary commits with a skip event.
    let before = state(&canwu);
    let relayed = revision(&before, "relayed", &[MERCHANT], &[]);
    let events = canwu.events().len();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PROVIDER,
            "relay",
            at,
            serde_json::to_value(&relayed).expect("request"),
        ))
        .expect("relay ingress");
    settle(&mut canwu, at);
    settle(&mut canwu, at);
    let skips: Vec<_> = canwu.events()[events..]
        .iter()
        .filter_map(ResourceIngressSkipV1::from_event)
        .collect();
    assert_eq!(skips.len(), 1, "{skips:?}");
    assert_eq!(
        skips[0].reason,
        ResourceIngressSkipReasonV1::FailedSettlementChecks
    );
    assert_eq!(
        skips[0].operation_key.as_ref(),
        Some(&relayed.operation_key)
    );
    let unrevised = state(&canwu);
    assert!(!unrevised.outcomes.contains_key(&relayed.operation_key));
    assert_eq!(unrevised.run_budget, before.run_budget);

    // Evidence of a record kind the plugin does not read is a recorded
    // rejection.
    let mut unread = revision(&unrevised, "unread", &[MERCHANT], &[]);
    unread.evidence = record_version("ledger", "entry");
    enqueue_resource_run_budget_revision(&mut canwu, at, &unread)
        .expect("the ledger entry is current");
    settle(&mut canwu, at);
    let after_unread = state(&canwu);
    let outcome = &after_unread.outcomes[&unread.operation_key];
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_authority"));
    assert_eq!(after_unread.run_budget, before.run_budget);

    // The host retires the merchant and adds the newcomer. An `Acquire` of
    // the merchant queued after it in the same boundary finds no partition;
    // the newcomer's single step in the next boundary activates.
    let swap = revision(&state(&canwu), "swap", &[MERCHANT], &[NEWCOMER]);
    enqueue_resource_run_budget_revision(&mut canwu, at, &swap).expect("revision ingress");
    enqueue_resource_completion_operation(
        &mut canwu,
        at,
        &ResourceCompletionOperationV1::Acquire(lease_request(MERCHANT, "late", at)),
    )
    .expect("completion ingress");
    settle(&mut canwu, at);
    let revised = state(&canwu);
    let outcome = &revised.outcomes[&swap.operation_key];
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(outcome.kind, ResourceOperationKind::ReviseRunBudget);
    assert_eq!(outcome.exact_evidence, vec![evidence("works")]);
    assert_eq!(
        revised.run_budget.revision,
        ResourceRevision::INITIAL.next().expect("revision")
    );
    assert!(
        !revised
            .holds_partition(&partition_key(MERCHANT))
            .expect("whole")
    );
    assert!(
        revised
            .holds_partition(&partition_key(NEWCOMER))
            .expect("whole")
    );
    // A completion operation's key derives from the identities it names.
    let late_key =
        ResourceCompletionOperationV1::Acquire(lease_request(MERCHANT, "late", at)).operation_key();
    let late = &revised.outcomes[&late_key];
    assert_eq!(late.status, ResourceOperationStatus::Rejected);
    assert_eq!(late.rejection_code.as_deref(), Some("invalid_authority"));

    let at = minute(2);
    let boundary = next_boundary(&canwu);
    enqueue_resource_completion_operation(
        &mut canwu,
        at,
        &ResourceCompletionOperationV1::SingleStepLease(single_step(
            NEWCOMER, "newcomer", at, boundary,
        )),
    )
    .expect("completion ingress");
    settle(&mut canwu, at);
    let settled = state(&canwu);
    assert!(
        settled
            .completion_leases
            .certificates
            .contains_key(&acquisition("newcomer"))
    );
    assert!(
        settled
            .admission_epoch(&partition_key(NEWCOMER))
            .expect("whole")
            .is_some()
    );
    settled.validate().expect("root after the revision");

    // Evidence the host cited while it was current, superseded before the
    // revision settles, is a recorded rejection.
    let superseded = revision(&settled, "superseded", &[], &[4]);
    enqueue_resource_run_budget_revision(&mut canwu, minute(4), &superseded)
        .expect("the evidence is current when the host queues the revision");
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PROVIDER,
            "bump",
            minute(3),
            serde_json::json!({ "record": "works" }),
        ))
        .expect("bump ingress");
    settle(&mut canwu, minute(3));
    assert_eq!(
        canwu
            .current_domain_record_version(&evidence("works").record)
            .expect("version read")
            .expect("works record")
            .version,
        2
    );
    settle(&mut canwu, minute(4));
    let last = state(&canwu);
    let outcome = &last.outcomes[&superseded.operation_key];
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some("version_conflict"));
    assert_eq!(last.run_budget, settled.run_budget);

    // A restored run and an exact replay hold the same root.
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let restored = from_resource_snapshot_json(&canwu.snapshot_json().expect("snapshot"), &plugins)
        .expect("checked restore");
    assert_eq!(state(&restored), last);
    let replayed =
        replay_resource_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(state(&replayed), last);
}

/// The resource records `canwu` holds, by reference.
fn resource_records(canwu: &Canwu) -> BTreeMap<String, DomainRecord> {
    canwu
        .domain_records()
        .filter(|record| record.owner == PLUGIN_NAME)
        .map(|record| (record.reference.to_string(), record.clone()))
        .collect()
}

fn live_bucket_of(key: &CompletionPartitionKeyV1) -> String {
    resource_live_bucket_reference(resource_live_bucket(
        &ResourceLiveRecordKeyV1::Partition(key.clone()),
        resource_live_bucket_count(&ResourceLimitsV1::canonical()),
    ))
    .into_untyped()
    .to_string()
}

fn is_live_bucket(reference: &str) -> bool {
    reference.contains("resource:live:")
}

/// Queues one completion operation, settles its boundary, and returns the
/// live buckets that boundary wrote.
fn step(
    canwu: &mut Canwu,
    at: SimTime,
    operation: ResourceCompletionOperationV1,
) -> BTreeSet<String> {
    let before = resource_records(canwu);
    enqueue_resource_completion_operation(canwu, at, &operation).expect("completion ingress");
    settle(canwu, at);
    resource_records(canwu)
        .into_iter()
        .filter(|(reference, record)| {
            is_live_bucket(reference) && before.get(reference) != Some(record)
        })
        .map(|(reference, _)| reference)
        .collect()
}

/// The stored entry of `key` in its live bucket.
fn stored_entry(canwu: &Canwu, key: &CompletionPartitionKeyV1) -> serde_json::Value {
    let bucket = resource_records(canwu)
        .remove(&live_bucket_of(key))
        .expect("the partition's bucket");
    let wanted =
        serde_json::to_value(ResourceLiveRecordKeyV1::Partition(key.clone())).expect("key encodes");
    bucket.payload["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["key"] == wanted)
        .cloned()
        .expect("the partition's entry")
}

#[test]
fn only_the_admitting_boundary_writes_a_live_bucket() {
    // Two more authorities whose partitions share one live bucket.
    let buckets = resource_live_bucket_count(&ResourceLimitsV1::canonical());
    let bucket_of = |value: u64| {
        resource_live_bucket(
            &ResourceLiveRecordKeyV1::Partition(partition_key(value)),
            buckets,
        )
    };
    let (first, second) = (10..10_000)
        .flat_map(|left| ((left + 1)..(left + 2_000)).map(move |right| (left, right)))
        .find(|(left, right)| bucket_of(*left) == bucket_of(*right))
        .expect("two partitions in one bucket");
    let mut root = world();
    let mut shared = revision(&root, "shared", &[], &[first, second]);
    shared.total_completion_units = 4_000_000;
    applied(&mut root, &shared);
    let resource_plugin = plugin();
    let mut canwu = runtime(121, &resource_plugin, root);
    let keeper_bucket = live_bucket_of(&partition_key(KEEPER));

    // The four steps: only the acquisition writes, and only the keeper's
    // bucket.
    let at = minute(1);
    let written = step(
        &mut canwu,
        at,
        ResourceCompletionOperationV1::Acquire(lease_request(KEEPER, "four", at)),
    );
    assert_eq!(written, BTreeSet::from([keeper_bucket.clone()]));
    assert!(
        state(&canwu)
            .completion_leases
            .acquisitions
            .contains_key(&acquisition("four"))
    );
    let boundary = next_boundary(&canwu);
    let operation = grant_request(&state(&canwu), "four", boundary);
    assert!(step(&mut canwu, at, operation).is_empty(), "grant");
    assert!(
        state(&canwu)
            .completion_leases
            .grants
            .contains_key(&grant("four")),
        "the grant applied"
    );
    let boundary = next_boundary(&canwu);
    let operation = prepare_request(&state(&canwu), "four", boundary);
    assert!(step(&mut canwu, at, operation).is_empty(), "prepare");
    assert_eq!(
        state(&canwu).completion_leases.grants[&grant("four")].state,
        CompletionGrantStateV1::Prepared,
        "the prepare applied"
    );
    let boundary = next_boundary(&canwu);
    let operation = activate_request(&state(&canwu), "four", at, boundary);
    assert!(step(&mut canwu, at, operation).is_empty(), "activate");
    assert!(
        state(&canwu)
            .completion_leases
            .certificates
            .contains_key(&acquisition("four")),
        "the activation applied"
    );

    // A lease given up before activation: its release writes none.
    let at = minute(3);
    let written = step(
        &mut canwu,
        at,
        ResourceCompletionOperationV1::Acquire(lease_request(KEEPER, "released", at)),
    );
    assert_eq!(written, BTreeSet::from([keeper_bucket.clone()]));
    let boundary = next_boundary(&canwu);
    let operation = grant_request(&state(&canwu), "released", boundary);
    assert!(step(&mut canwu, at, operation).is_empty(), "grant");
    assert!(
        state(&canwu)
            .completion_leases
            .grants
            .contains_key(&grant("released")),
        "the grant applied"
    );
    let operation = release_request(&state(&canwu), "released");
    assert!(step(&mut canwu, at, operation).is_empty(), "release");
    assert!(
        !state(&canwu)
            .completion_leases
            .acquisitions
            .contains_key(&acquisition("released")),
        "the release applied: the lease left the book"
    );

    // The single step writes the bucket of its partition once.
    let at = minute(5);
    let boundary = next_boundary(&canwu);
    let written = step(
        &mut canwu,
        at,
        ResourceCompletionOperationV1::SingleStepLease(single_step(
            MERCHANT, "single", at, boundary,
        )),
    );
    assert_eq!(
        written,
        BTreeSet::from([live_bucket_of(&partition_key(MERCHANT))])
    );

    // Of two partitions in one bucket, an admission of the first leaves the
    // second's entry byte-identical.
    let untouched = stored_entry(&canwu, &partition_key(second));
    let at = minute(7);
    let boundary = next_boundary(&canwu);
    let written = step(
        &mut canwu,
        at,
        ResourceCompletionOperationV1::SingleStepLease(single_step(
            first,
            "same-bucket",
            at,
            boundary,
        )),
    );
    assert_eq!(
        written,
        BTreeSet::from([live_bucket_of(&partition_key(first))])
    );
    assert_eq!(stored_entry(&canwu, &partition_key(second)), untouched);
    assert_ne!(
        stored_entry(&canwu, &partition_key(first))["payload"]["record"]["epoch"],
        serde_json::Value::Null,
        "the admitted partition's entry carries its admission epoch"
    );
    validate_resource_runtime(&canwu).expect("the kept evidence exists");
}

#[test]
fn a_partial_reader_attaches_partitions_through_load_run_and_reattach() {
    let resource_plugin = plugin();
    let mut canwu = runtime(122, &resource_plugin, world());
    let at = minute(1);
    let boundary = next_boundary(&canwu);
    enqueue_resource_completion_operation(
        &mut canwu,
        at,
        &ResourceCompletionOperationV1::SingleStepLease(single_step(KEEPER, "read", at, boundary)),
    )
    .expect("completion ingress");
    settle(&mut canwu, at);
    let whole = state(&canwu);

    let mut reader = ResourceRootReader::load_run(&canwu)
        .expect("reader")
        .expect("resource root");
    reader
        .attach_live([
            ResourceLiveRecordKeyV1::Partition(partition_key(KEEPER)),
            ResourceLiveRecordKeyV1::Partition(partition_key(NEWCOMER)),
        ])
        .expect("attach partitions");
    {
        let partial = reader.state();
        assert_eq!(partial.partition_count(), 2);
        assert_eq!(
            partial.partition(&partition_key(KEEPER)).expect("attached"),
            whole.partition(&partition_key(KEEPER)).expect("whole")
        );
        assert_eq!(
            partial
                .admission_epoch(&partition_key(KEEPER))
                .expect("attached"),
            whole
                .admission_epoch(&partition_key(KEEPER))
                .expect("whole")
        );
        assert_eq!(
            partial.partition(&partition_key(NEWCOMER)).expect("absent"),
            None,
            "a partition the root does not hold answers none"
        );
        assert!(
            partial.partition(&partition_key(MERCHANT)).is_err(),
            "a partition never attached is not answered"
        );
        assert!(partial.holds_partition(&partition_key(MERCHANT)).is_err());
        assert_eq!(partial.run_budget.revision, whole.run_budget.revision);
        assert_eq!(
            partial.run_budget.semantic_digest, whole.run_budget.semantic_digest,
            "the digest covers the header a partial root holds"
        );
    }
    let kept = reader.keep();

    // The host revises the budget; the re-attached reader repeats the
    // requests and reads what changed.
    let at = minute(2);
    let swap = revision(&whole, "swap", &[MERCHANT], &[NEWCOMER]);
    enqueue_resource_run_budget_revision(&mut canwu, at, &swap).expect("revision ingress");
    settle(&mut canwu, at);
    let whole = state(&canwu);
    let reattached = ResourceRootReader::reattach(&canwu, kept)
        .expect("reattach")
        .expect("resource root");
    let partial = reattached.state();
    assert_eq!(
        partial
            .partition(&partition_key(NEWCOMER))
            .expect("attached"),
        Some(&partition(NEWCOMER))
    );
    assert_eq!(
        partial.partition(&partition_key(KEEPER)).expect("attached"),
        whole.partition(&partition_key(KEEPER)).expect("whole")
    );
    assert_eq!(partial.run_budget.revision, whole.run_budget.revision);
    let mut fresh = ResourceRootReader::load_run(&canwu)
        .expect("reader")
        .expect("resource root");
    fresh
        .attach_live([
            ResourceLiveRecordKeyV1::Partition(partition_key(KEEPER)),
            ResourceLiveRecordKeyV1::Partition(partition_key(NEWCOMER)),
        ])
        .expect("attach partitions");
    assert_eq!(fresh.state(), partial);
}

#[test]
fn a_tampered_partition_split_is_refused() {
    let resource_plugin = plugin();
    let install = |records: Vec<DomainRecord>| {
        let scenario = Scenario::new(SimTime::EPOCH, Vec::new())
            .with_domain_records(fixture_records().into_iter().chain(records).collect());
        Canwu::new_with_plugins(4, scenario, &[&resource_plugin, &ProviderPlugin]).map(|_| ())
    };
    let mut leased = world();
    lease_and_release(&mut leased, KEEPER, "tamper");
    let whole_budget = serde_json::to_value(&leased.run_budget).expect("budget encodes");
    let whole_book = serde_json::to_value(&leased.completion_leases).expect("book encodes");
    let records = leased.clone().into_records().expect("the root persists");
    install(records.clone()).expect("the untouched root installs");
    let live_at = records
        .iter()
        .position(|record| record.reference.id == "resource:runtime:v1")
        .expect("the live root");
    assert_eq!(
        records[live_at].payload["run_budget"]["partitions"],
        serde_json::json!([]),
        "the live root holds no partition"
    );

    // Partitions inline in the live root.
    let mut inline_partitions = records.clone();
    inline_partitions[live_at].payload["run_budget"]["partitions"] =
        whole_budget["partitions"].clone();
    // Epochs inline in the live root.
    let mut inline_epochs = records.clone();
    inline_epochs[live_at].payload["completion_leases"]["epochs"] = whole_book["epochs"].clone();
    // A ledger that miscounts the partitions.
    let mut miscounted = records.clone();
    miscounted[live_at].payload["live"]["partitions"] = serde_json::json!(3);
    // A ledger that miscounts the epochs (more than the stored entries
    // hold), refused because it differs from the recount.
    let mut epochs_miscounted = records.clone();
    epochs_miscounted[live_at].payload["live"]["admission_epochs"] = serde_json::json!(3);
    for forged in [
        inline_partitions,
        inline_epochs,
        miscounted,
        epochs_miscounted,
    ] {
        let error = install(forged).expect_err("a tampered split root is refused");
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
    }

    // An epoch without its partition does not validate or persist.
    let mut orphaned = leased.clone();
    orphaned
        .run_budget
        .partitions
        .retain(|partition| !partition.has_key(&partition_key(KEEPER)));
    orphaned.run_budget.semantic_digest.clear();
    orphaned.run_budget = orphaned.run_budget.clone().seal().expect("reseal");
    assert!(orphaned.validate().is_err());
    assert!(orphaned.into_records().is_err());

    // A root that holds partitions persists only as several records.
    assert!(leased.into_record().is_err());
}

#[test]
fn a_continuation_checkpoint_restores_the_split_root_and_continues_in_lockstep() {
    let resource_plugin = plugin();
    let mut full = runtime(123, &resource_plugin, world());

    // The keeper's lease puts its epoch in its partition's bucket.
    let at = minute(1);
    let boundary = next_boundary(&full);
    enqueue_resource_completion_operation(
        &mut full,
        at,
        &ResourceCompletionOperationV1::SingleStepLease(single_step(
            KEEPER,
            "ckpt-keeper",
            at,
            boundary,
        )),
    )
    .expect("completion ingress");
    settle(&mut full, at);

    // A revision retires the merchant, whose bucket holds nothing else, and
    // adds the newcomer.
    let at = minute(2);
    let swap = revision(&state(&full), "ckpt-swap", &[MERCHANT], &[NEWCOMER]);
    enqueue_resource_run_budget_revision(&mut full, at, &swap).expect("revision ingress");
    settle(&mut full, at);
    assert_eq!(
        state(&full).outcomes[&swap.operation_key].status,
        ResourceOperationStatus::Applied
    );
    assert_eq!(
        resource_records(&full)[&live_bucket_of(&partition_key(MERCHANT))].payload["entries"],
        serde_json::json!([]),
        "the revision emptied the merchant's bucket"
    );

    // At the cut, a revision that adds another authority and an `Acquire` of
    // the retired merchant are pending.
    // The run first settles its own report wakes due before the cut.
    let at = minute(5);
    catch_up(&mut full, at);
    let pending = revision(&state(&full), "ckpt-pending", &[], &[4]);
    let pending_revision =
        enqueue_resource_run_budget_revision(&mut full, at, &pending).expect("revision ingress");
    let late_acquire =
        ResourceCompletionOperationV1::Acquire(lease_request(MERCHANT, "ckpt-late", at));
    let pending_acquire = enqueue_resource_completion_operation(&mut full, at, &late_acquire)
        .expect("completion ingress");
    let (checkpoint, hash) = full
        .continuation_checkpoint()
        .expect("the continuation checkpoint builds");
    for receipt in [&pending_revision, &pending_acquire] {
        assert!(
            checkpoint.pending_ingress.contains(&receipt.ingress_id),
            "the revision and the Acquire are pending"
        );
    }
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let mut resumed =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("the split root restores");
    assert_eq!(resumed.checkpoint_hash(), full.checkpoint_hash());
    assert_eq!(
        resumed.authoritative_state_hash().expect("state hash"),
        full.authoritative_state_hash().expect("state hash")
    );
    assert_eq!(state(&resumed), state(&full));

    // Both runs settle the pending work alike: the revision applies and the
    // retired authority's `Acquire` finds no partition.
    assert_eq!(
        resumed
            .settle_boundary(BoundaryRequest::at(at))
            .expect("resumed boundary"),
        full.settle_boundary(BoundaryRequest::at(at))
            .expect("full boundary")
    );
    assert_eq!(resumed.checkpoint_hash(), full.checkpoint_hash());
    for root in [state(&resumed), state(&full)] {
        assert_eq!(
            root.outcomes[&pending.operation_key].status,
            ResourceOperationStatus::Applied
        );
        let late = &root.outcomes[&late_acquire.operation_key()];
        assert_eq!(late.status, ResourceOperationStatus::Rejected);
        assert_eq!(late.rejection_code.as_deref(), Some("invalid_authority"));
        assert!(root.holds_partition(&partition_key(4)).expect("whole"));
    }

    // And a later lease of the newcomer.
    let at = minute(6);
    let boundary = next_boundary(&full);
    let lease = ResourceCompletionOperationV1::SingleStepLease(single_step(
        NEWCOMER,
        "ckpt-newcomer",
        at,
        boundary,
    ));
    enqueue_resource_completion_operation(&mut full, at, &lease).expect("completion ingress");
    enqueue_resource_completion_operation(&mut resumed, at, &lease).expect("completion ingress");
    assert_eq!(
        resumed
            .settle_boundary(BoundaryRequest::at(at))
            .expect("resumed boundary"),
        full.settle_boundary(BoundaryRequest::at(at))
            .expect("full boundary")
    );
    assert_eq!(resumed.checkpoint_hash(), full.checkpoint_hash());
    assert_eq!(
        resumed.authoritative_state_hash().expect("state hash"),
        full.authoritative_state_hash().expect("state hash")
    );
    let settled = state(&full);
    assert_eq!(state(&resumed), settled);
    assert!(
        settled
            .completion_leases
            .certificates
            .contains_key(&acquisition("ckpt-newcomer"))
    );
}

/// The live bucket of one key of each kind at the canonical bucket count,
/// pinned: placement hashes each key's text (an identity, a holder's
/// encoding, or a partition's holder and namespace), so a change to that text
/// would move stored records between buckets without a format change.
#[test]
fn live_bucket_placement_is_pinned() {
    let buckets = resource_live_bucket_count(&ResourceLimitsV1::canonical());
    assert_eq!(buckets, 32_768);
    let pinned = [
        (ResourceLiveRecordKeyV1::Account(account("granary")), 16_031),
        (
            ResourceLiveRecordKeyV1::AccessGrant(
                ResourceAccessGrantId::new("fixture:access-grant:market").expect("grant"),
            ),
            3_824,
        ),
        (
            ResourceLiveRecordKeyV1::Definition(
                ResourceDefinitionRevisionId::new("fixture:resource:grain:v1").expect("id"),
            ),
            24_641,
        ),
        (
            ResourceLiveRecordKeyV1::Unit(
                ResourceUnitRevisionId::new("fixture:unit:measure:v1").expect("unit"),
            ),
            27_244,
        ),
        (ResourceLiveRecordKeyV1::Custodian(holder(KEEPER)), 30_053),
        (
            ResourceLiveRecordKeyV1::Partition(partition_key(KEEPER)),
            8_427,
        ),
    ];
    for (key, bucket) in pinned {
        assert_eq!(resource_live_bucket(&key, buckets), bucket, "{key:?}");
    }
}
