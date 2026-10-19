//! Downstream gap G-65: a loss may take part of a transfer's escrow.
//!
//! In a plugin-owned run, the source custodian records a partial loss of an
//! open transfer by tracked command. The transfer keeps its state and the
//! rest of its escrow, its revision advances, the loss counts as admitted
//! loss, and the lease the transfer was started under stays until the
//! transfer's terminal disposition. A zero quantity, the whole escrow, a stale
//! revision, and a terminal transfer are recorded rejections, and a holder who
//! does not control the transfer is refused at admission; no boundary fails.
//! The loss is archived while its transfer is still open. Local acceptance
//! then credits the destination with what is left and the demand with what is
//! accepted, so the loss reopens that much of it; the root conserves, and a
//! restored run and an exact replay agree.
//!
//! A second fixture takes a partial loss in every open state, then loses the
//! rest with a `Lose` disposition: `lost` totals both.
//!
//! In two more plugin-owned runs, a demand cancelled or expired while its
//! transfer is in flight keeps its status when what is left after a partial
//! loss is accepted, and an allocation pass counts escrow in flight as drawn:
//! it neither allocates it again nor rejects the demand for its minimum, and
//! after a partial loss it allocates exactly what was lost. An account loss
//! that names an archived loss's identity is refused.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, EvidenceRef, Issuer,
    KnowledgeHolderRef, PayloadSchema, PersonId, PluginArchiveObjectProvider, PluginRegistrar,
    Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

const NAMESPACE: &str = "fixture.resource";
const EVIDENCE_NAMESPACE: &str = "fixture.provider";
const SOURCE: u64 = 1;
const DESTINATION: u64 = 2;

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

fn evidence(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, "evidence", id),
        version: 1,
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

/// Owner of the loss causes and handover records the fixture cites.
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
        "0000000000000000000000000000000000000000000000000000000000000065"
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

/// A source account of 100, a spare account of 50 of the same custodian, and
/// an empty destination account in one place, and room for both custodians'
/// leases.
fn world() -> ResourceState {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![partition(holder(SOURCE)), partition(holder(DESTINATION))],
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
        ("source", SOURCE, 100),
        ("spare", SOURCE, 50),
        ("destination", DESTINATION, 0),
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

/// A demand of the source custodian on its own account.
fn demand(state: &ResourceState, name: &str, quantity: u64, at: SimTime) -> ResourceDemand {
    let source = &state.accounts[&account("source")];
    ResourceDemand {
        id: ResourceDemandId::new(format!("fixture:demand:{name}")).expect("demand"),
        revision: ResourceRevision::INITIAL,
        requester: holder(SOURCE),
        source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![account("source")]),
        resource_revision: source.resource_revision.clone(),
        unit_revision: source.unit_revision.clone(),
        requested: quantity,
        fulfilled: 0,
        minimum_useful: 1,
        partial_fulfillment: PartialFulfillmentPolicy::AcceptPartial,
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

/// The targets a partial loss of `transfer` at its current revision locks.
fn loss_targets(
    state: &ResourceState,
    transfer: &ResourceTransferId,
    cause: &DomainRecordVersionRef,
) -> Vec<CompletionLockedTargetV1> {
    vec![
        CompletionLockedTargetV1::Transfer {
            id: transfer.clone(),
            revision: state.transfers[transfer].revision,
        },
        CompletionLockedTargetV1::ExternalRecord {
            version: cause.clone(),
        },
    ]
}

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

fn transfer_loss(
    loss: &str,
    transfer: &ResourceTransferId,
    expected_transfer_revision: ResourceRevision,
    quantity: u64,
    cause: &DomainRecordVersionRef,
    at: SimTime,
    completion_certificate: CompletionLeaseActivationCertificateV1,
) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::RecordTransferLoss(ResourceTransferLossRequestV1 {
        operation_key: key(&format!("fixture:loss:{loss}")),
        loss_id: ResourceLossId::new(format!("fixture:loss:{loss}")).expect("loss"),
        transfer: transfer.clone(),
        expected_transfer_revision,
        quantity,
        cause: EvidenceRef::DomainRecordVersion(cause.clone()),
        at,
        completion_certificate,
    })
}

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

/// A content-addressed archive store shared by the host and the runtime.
#[derive(Clone, Default)]
struct MemoryArchive {
    objects: Arc<RwLock<ArchiveObjects>>,
    retention: Arc<RwLock<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
}

impl ResourceArchiveStore for MemoryArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .write()
            .unwrap()
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
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .write()
            .unwrap()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .write()
            .unwrap()
            .get_mut(handle_id)
            .ok_or_else(|| ResourceError::NotFound("retention handle unavailable".to_owned()))?
            .phase = phase;
        Ok(())
    }
}

impl PluginArchiveObjectProvider for MemoryArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(self
            .objects
            .read()
            .unwrap()
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

/// A plugin-owned run with an archive and the next tracked command id.
struct Run {
    canwu: Canwu,
    archive: MemoryArchive,
    commands: u64,
}

impl Run {
    /// Submits one tracked command from `actor` and, when admitted, settles
    /// it. Returns the admission outcome.
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

    /// Settles one admitted command and returns the outcome under its key.
    fn settled(
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
        state(&self.canwu).outcomes[&operation_key].clone()
    }

    fn applied(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
    ) -> ResourceOperationOutcome {
        let outcome = self.settled(actor, at, request);
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{:?}",
            outcome.rejection_reason
        );
        outcome
    }

    /// A recorded rejection with `code` that changed nothing but its outcome.
    fn refused(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
        code: &str,
    ) {
        let before = state(&self.canwu);
        let outcome = self.settled(actor, at, request);
        assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{code}");
        assert_eq!(outcome.kind, ResourceOperationKind::Loss);
        assert_eq!(
            outcome.rejection_code.as_deref(),
            Some(code),
            "{:?}",
            outcome.rejection_reason
        );
        let after = state(&self.canwu);
        assert_eq!(after.transfers, before.transfers, "{code}");
        assert_eq!(after.losses, before.losses, "{code}");
        assert_eq!(after.conservation, before.conservation, "{code}");
    }

    /// Activates one single-step lease through the completion ingress.
    fn lease(
        &mut self,
        lease_holder: u64,
        suffix: &str,
        operation_key: &ResourceOperationKey,
        targets: &[CompletionLockedTargetV1],
        at: SimTime,
    ) -> CompletionLeaseActivationCertificateV1 {
        let operation =
            ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
                lease: lease_request(lease_holder, suffix, operation_key, targets, at),
                grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}"))
                    .expect("grant"),
                target_versions: targets.to_vec(),
                current_boundary: next_boundary(&self.canwu),
            });
        enqueue_resource_completion_operation(&mut self.canwu, at, &operation)
            .expect("completion ingress");
        settle(&mut self.canwu, at);
        state(&self.canwu).completion_leases.certificates[&acquisition(suffix)].clone()
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
        assert!(state(&self.canwu).terminal_archive_candidates.len() <= 1);
    }
}

#[test]
fn a_partial_loss_keeps_the_transfer_open_and_its_demand_is_credited_with_what_is_accepted() {
    let cause = evidence("cause");
    let handover = evidence("handover");
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(SOURCE)),
            EntityRef::Person(PersonId::new(DESTINATION)),
        ],
    )
    .with_domain_records(
        [&cause, &handover]
            .into_iter()
            .map(evidence_record)
            .chain(world().into_records().expect("resource records"))
            .collect(),
    );
    let archive = MemoryArchive::default();
    let mut canwu = Canwu::new_with_plugins(65, scenario, &plugins).expect("runtime");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    let mut run = Run {
        canwu,
        archive,
        commands: 0,
    };

    // A transfer of 30 for a demand of 30, waiting for dispatch.
    let at = minute(1);
    let request = demand(&state(&run.canwu), "delivery", 30, at);
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
            operation_key: key("fixture:submit:delivery"),
            demand: request,
        }),
    );
    let expected_state_revision = state(&run.canwu).state_revision;
    enqueue_resource_allocation(
        &mut run.canwu,
        at,
        &holder(SOURCE),
        &ResourceAllocationRequestV1 {
            operation_key: key("fixture:allocate:delivery"),
            expected_state_revision,
            at,
            candidate_limit: 8,
        },
    )
    .expect("allocation ingress");
    settle(&mut run.canwu, at);
    let current = state(&run.canwu);
    let leg = leg_of(&current, "delivery");
    let start_key = key("fixture:start:delivery");
    let start = run.lease(
        SOURCE,
        "start",
        &start_key,
        &start_targets(&current, &leg),
        at,
    );
    let transfer = ResourceTransferId::new("fixture:transfer:delivery").expect("transfer");
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: transfer.clone(),
            allocation: (&leg).into(),
            expected_account_revision: current.accounts[&leg.account].revision,
            destination: Some(account("destination")),
            at,
            completion_certificate: start,
        }),
    );

    // Ten units are lost; the transfer stays open with the other twenty.
    let at = minute(2);
    let opened = state(&run.canwu);
    let opened_revision = opened.transfers[&transfer].revision;
    let loss_key = key("fixture:loss:first");
    let certificate = run.lease(
        SOURCE,
        "loss",
        &loss_key,
        &loss_targets(&opened, &transfer, &cause),
        at,
    );
    let first = transfer_loss(
        "first",
        &transfer,
        opened_revision,
        10,
        &cause,
        at,
        certificate.clone(),
    );
    let outcome = run.applied(SOURCE, at, first.clone());
    assert_eq!(outcome.kind, ResourceOperationKind::Loss);
    assert_eq!((outcome.quantity, outcome.remainder), (10, 20));
    assert_eq!(outcome.exact_evidence, vec![cause.clone()]);
    let lost = state(&run.canwu);
    let open = &lost.transfers[&transfer];
    assert_eq!(open.state, ResourceTransferState::PendingDispatch);
    assert_eq!((open.escrow, open.lost, open.quantity), (20, 10, 30));
    assert_eq!(open.revision, opened_revision.next().expect("revision"));
    let loss = &lost.losses[&ResourceLossId::new("fixture:loss:first").expect("loss")];
    assert_eq!(
        (loss.account.as_ref(), loss.transfer.as_ref(), loss.quantity),
        (None, Some(&transfer), 10)
    );
    assert_eq!(lost.conservation.admitted_loss, 10);
    // The loss completed its own lease only: the lease the transfer was
    // started under still holds the transfer until its terminal disposition.
    assert!(
        !lost
            .completion_leases
            .acquisitions
            .contains_key(&acquisition("loss"))
    );
    assert!(
        lost.completion_leases
            .acquisitions
            .contains_key(&acquisition("start"))
    );
    assert_eq!(lost.accounts[&account("source")].balance, 70);
    lost.validate().expect("root after a partial loss");

    // Refusals are recorded, and no boundary fails.
    for (name, quantity, revision, code) in [
        ("zero", 0, open.revision, "invalid_definition"),
        ("whole", 20, open.revision, "invalid_lifecycle"),
        ("stale", 5, opened_revision, "version_conflict"),
    ] {
        run.refused(
            SOURCE,
            at,
            transfer_loss(
                name,
                &transfer,
                revision,
                quantity,
                &cause,
                at,
                certificate.clone(),
            ),
            code,
        );
    }
    let outsider = run.command(
        DESTINATION,
        at,
        transfer_loss(
            "outsider",
            &transfer,
            open.revision,
            5,
            &cause,
            at,
            certificate.clone(),
        ),
    );
    assert!(
        matches!(outsider, CommandAttemptOutcome::Rejected { .. }),
        "only the transfer's controller records its loss: {outsider:?}"
    );

    // The loss leaves the root with the archive while its transfer is open.
    run.archive_all();
    let archived = state(&run.canwu);
    let loss_id = ResourceLossId::new("fixture:loss:first").expect("loss");
    assert!(!archived.losses.contains_key(&loss_id));
    assert!(archived.active_transfers.contains(&transfer));
    let record = archived_resource_terminal_record(
        &archived,
        &run.archive,
        &ResourceTerminalRecordKeyV1::Loss(loss_id.clone()),
    )
    .expect("archive lookup")
    .expect("archived loss");
    assert!(matches!(
        record.payload,
        ResourceTerminalArchivePayloadV1::Loss(ref value)
            if value.transfer.as_ref() == Some(&transfer) && value.quantity == 10
    ));
    // A resend is answered from the archive and settles nothing.
    let resent = run.command(SOURCE, at, first);
    assert!(matches!(resent, CommandAttemptOutcome::Accepted { .. }));
    assert_eq!(
        state(&run.canwu).transfers[&transfer],
        archived.transfers[&transfer]
    );

    // The destination accepts the twenty that are left.
    let current = state(&run.canwu);
    let transfer_revision = current.transfers[&transfer].revision;
    let destination_revision = current.accounts[&account("destination")].revision;
    let accept_key = key("fixture:accept:delivery");
    let accept = run.lease(
        DESTINATION,
        "accept",
        &accept_key,
        &[
            CompletionLockedTargetV1::Transfer {
                id: transfer.clone(),
                revision: transfer_revision,
            },
            CompletionLockedTargetV1::Account {
                id: account("destination"),
                revision: destination_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: handover.clone(),
            },
        ],
        at,
    );
    let accepted = run.applied(
        DESTINATION,
        at,
        ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
            operation_key: accept_key,
            transfer: transfer.clone(),
            expected_transfer_revision: transfer_revision,
            at,
            disposition: ResourceTransferDispositionV1::AcceptLocal {
                destination: account("destination"),
                expected_destination_revision: destination_revision,
                handover_evidence: handover.clone(),
            },
            exact_transport_evidence: None,
            completion_certificate: accept,
        }),
    );
    assert_eq!((accepted.quantity, accepted.remainder), (20, 10));
    let settled = state(&run.canwu);
    let done = &settled.transfers[&transfer];
    assert_eq!(done.state, ResourceTransferState::Accepted);
    assert_eq!(
        (done.escrow, done.accepted, done.lost, done.quantity),
        (0, 20, 10, 30)
    );
    assert_eq!(settled.accounts[&account("source")].balance, 70);
    assert_eq!(settled.accounts[&account("destination")].balance, 20);
    assert_eq!(settled.conservation.admitted_loss, 10);
    settled.validate_conservation().expect("conserved");
    // The demand is credited with the twenty accepted: the lost ten reopen.
    let delivery = &settled.demands[&leg.demand];
    assert_eq!(delivery.status, DemandStatus::PartiallyFulfilled);
    assert_eq!((delivery.fulfilled, delivery.remainder()), (20, 10));
    assert!(settled.dirty_demands.contains(&leg.demand));
    let fulfillment = latest_resource_fulfillment(&run.canwu, &leg.demand).expect("fulfillment");
    assert_eq!(
        (fulfillment.consumed_quantity, fulfillment.remainder),
        (20, 10)
    );
    // Both leases have finished and hold nothing.
    for suffix in ["start", "accept"] {
        assert!(
            !settled
                .completion_leases
                .acquisitions
                .contains_key(&acquisition(suffix))
        );
    }
    assert!(settled.completion_leases.target_locks.is_empty());

    // A terminal transfer loses nothing more.
    run.refused(
        SOURCE,
        at,
        transfer_loss(
            "terminal",
            &transfer,
            done.revision,
            5,
            &cause,
            at,
            certificate,
        ),
        "version_conflict",
    );

    // The identity of the archived loss is not taken again, by an account
    // loss either: the archive is read for it.
    let at = minute(3);
    let current = state(&run.canwu);
    let source_revision = current.accounts[&account("source")].revision;
    let reuse_key = key("fixture:loss:reuse");
    let reuse = run.lease(
        SOURCE,
        "reuse",
        &reuse_key,
        &[
            CompletionLockedTargetV1::Account {
                id: account("source"),
                revision: source_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: cause.clone(),
            },
        ],
        at,
    );
    run.refused(
        SOURCE,
        at,
        ResourceOperationRequestV1::RecordLoss(ResourceAccountLossRequestV1 {
            operation_key: reuse_key,
            loss_id: loss_id.clone(),
            account: account("source"),
            expected_account_revision: source_revision,
            quantity: 5,
            cause: EvidenceRef::DomainRecordVersion(cause.clone()),
            allow_protected: false,
            at,
            completion_certificate: reuse,
        }),
        "idempotency_conflict",
    );
    assert_eq!(state(&run.canwu).accounts[&account("source")].balance, 70);
    run.release("reuse", at);

    // A restored run and an exact replay hold the same root.
    let settled = state(&run.canwu);
    let restored = from_resource_snapshot_json_with_archive_store(
        &run.canwu.snapshot_json().expect("snapshot"),
        &plugins,
        Arc::new(run.archive.clone()),
    )
    .expect("checked restore");
    assert_eq!(state(&restored), settled);
    let replayed = replay_resource_from_journal_with_archive_store(
        &plugins,
        &run.canwu.replay_journal(),
        Arc::new(run.archive.clone()),
    )
    .expect("exact replay");
    assert_eq!(state(&replayed), settled);
}

/// Activates one four-step lease held by the source custodian in memory.
fn certificate(
    state: &mut ResourceState,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: Vec<CompletionLockedTargetV1>,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
    let lease = lease_request(SOURCE, suffix, operation_key, &targets, at);
    let id = lease.id.clone();
    let digest = lease.eligibility_envelope.digest.clone();
    let grant = CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}")).expect("grant");
    let apply = |state: &mut ResourceState, operation| {
        let outcome = state
            .apply_operation(&ResourceOperationRequestV1::Completion(operation))
            .expect("completion");
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{:?}",
            outcome.rejection_reason
        );
    };
    apply(state, ResourceCompletionOperationV1::Acquire(lease));
    let revision = state.completion_leases.acquisitions[&id].revision;
    apply(
        state,
        ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
            grant_id: grant.clone(),
            acquisition: id.clone(),
            expected_acquisition_revision: revision,
            owner_plugin: PLUGIN_NAME.to_owned(),
            target_versions: targets,
            current_boundary: 10,
        }),
    );
    apply(
        state,
        ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
            acquisition: id.clone(),
            expected_acquisition_revision: state.completion_leases.acquisitions[&id].revision,
            grant: grant.clone(),
            expected_grant_revision: state.completion_leases.grants[&grant].revision,
            current_boundary: 11,
            eligibility_envelope_digest: digest.clone(),
        }),
    );
    apply(
        state,
        ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
            acquisition: id.clone(),
            expected_acquisition_revision: state.completion_leases.acquisitions[&id].revision,
            grant: grant.clone(),
            expected_grant_revision: state.completion_leases.grants[&grant].revision,
            at,
            current_boundary: 12,
            eligibility_envelope_digest: digest,
        }),
    );
    state.completion_leases.certificates[&id].clone()
}

#[test]
fn a_partial_loss_is_taken_in_every_open_state_and_the_whole_loss_takes_the_rest() {
    let cause = evidence("cause");
    let transport = TransportExecutionLink {
        execution: canwu_api::TransportExecutionId(1),
        itinerary_revision: canwu_api::ItineraryRevisionId(1),
        leg_execution: Some(canwu_api::LegExecutionId(1)),
        handoff: None,
        capacity_booking: None,
    };
    // How each transfer reaches its open state: the advances it makes, or a
    // cancellation.
    let paths: [(&str, &[TransferProgressV1], bool, ResourceTransferState); 5] = [
        (
            "pending",
            &[],
            false,
            ResourceTransferState::PendingDispatch,
        ),
        (
            "transit",
            &[TransferProgressV1::InTransit],
            false,
            ResourceTransferState::InTransit,
        ),
        (
            "arrival",
            &[
                TransferProgressV1::InTransit,
                TransferProgressV1::ArrivalPending,
            ],
            false,
            ResourceTransferState::ArrivalPending,
        ),
        (
            "return",
            &[TransferProgressV1::ReturnPending],
            false,
            ResourceTransferState::ReturnPending,
        ),
        (
            "cancelled",
            &[TransferProgressV1::InTransit],
            true,
            ResourceTransferState::ReturnPending,
        ),
    ];
    for (name, advances, cancel, open_state) in paths {
        let at = minute(1);
        let mut root = world();
        root.install_demand(demand(&root, name, 30, SimTime::EPOCH))
            .expect("demand");
        let revision = root.state_revision;
        root.apply_operation(&ResourceOperationRequestV1::Allocate(
            ResourceAllocationRequestV1 {
                operation_key: key(&format!("fixture:allocate:{name}")),
                expected_state_revision: revision,
                at: SimTime::EPOCH,
                candidate_limit: 8,
            },
        ))
        .expect("allocation");
        let leg = leg_of(&root, name);
        let start_key = key(&format!("fixture:start:{name}"));
        let targets = start_targets(&root, &leg);
        let start = certificate(&mut root, &format!("start-{name}"), &start_key, targets, at);
        let transfer = ResourceTransferId::new(format!("fixture:transfer:{name}")).expect("id");
        let source_revision = root.accounts[&leg.account].revision;
        root.apply_operation(&ResourceOperationRequestV1::BeginTransfer(
            ResourceTransferStartRequestV1 {
                operation_key: start_key,
                transfer_id: transfer.clone(),
                allocation: (&leg).into(),
                expected_account_revision: source_revision,
                destination: Some(account("destination")),
                at,
                completion_certificate: start,
            },
        ))
        .expect("transfer start");
        for (index, progress) in advances.iter().enumerate() {
            let outcome = root
                .apply_operation(&ResourceOperationRequestV1::AdvanceTransfer(
                    ResourceTransferProgressRequestV1 {
                        operation_key: key(&format!("fixture:advance:{name}:{index}")),
                        transfer: transfer.clone(),
                        expected_transfer_revision: root.transfers[&transfer].revision,
                        progress: *progress,
                        transport: transport.clone(),
                        transport_evidence: evidence(&format!("transport-{index}")),
                    },
                ))
                .expect("advance");
            assert_eq!(outcome.status, ResourceOperationStatus::Applied, "{name}");
        }
        if cancel {
            root.apply_operation(&ResourceOperationRequestV1::CancelTransfer(
                ResourceTransferCancellationRequestV1 {
                    operation_key: key(&format!("fixture:cancel:{name}")),
                    transfer: transfer.clone(),
                    expected_transfer_revision: root.transfers[&transfer].revision,
                    at,
                },
            ))
            .expect("cancellation");
        }
        assert_eq!(root.transfers[&transfer].state, open_state, "{name}");

        // One holder takes one lease a minute.
        let at = minute(2);
        let loss_key = key(&format!("fixture:loss:{name}"));
        let targets = loss_targets(&root, &transfer, &cause);
        let lease = certificate(&mut root, &format!("loss-{name}"), &loss_key, targets, at);
        let revision = root.transfers[&transfer].revision;
        let outcome = root
            .apply_operation(&transfer_loss(
                name, &transfer, revision, 5, &cause, at, lease,
            ))
            .expect("partial loss");
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{name}: {:?}",
            outcome.rejection_reason
        );
        let open = &root.transfers[&transfer];
        assert_eq!(open.state, open_state, "{name}");
        assert_eq!((open.escrow, open.lost), (25, 5), "{name}");
        root.validate().expect("partial loss");

        // The whole loss takes the rest, and `lost` totals both.
        let at = minute(3);
        let lose_key = key(&format!("fixture:lose:{name}"));
        let targets = loss_targets(&root, &transfer, &cause);
        let lease = certificate(&mut root, &format!("lose-{name}"), &lose_key, targets, at);
        let outcome = root
            .apply_operation(&ResourceOperationRequestV1::CompleteTransfer(
                ResourceTransferDispositionRequestV1 {
                    operation_key: lose_key,
                    transfer: transfer.clone(),
                    expected_transfer_revision: root.transfers[&transfer].revision,
                    at,
                    disposition: ResourceTransferDispositionV1::Lose {
                        loss_id: ResourceLossId::new(format!("fixture:lose:{name}")).expect("loss"),
                        cause: EvidenceRef::DomainRecordVersion(cause.clone()),
                    },
                    exact_transport_evidence: None,
                    completion_certificate: lease,
                },
            ))
            .expect("whole loss");
        assert_eq!(outcome.status, ResourceOperationStatus::Applied, "{name}");
        assert_eq!(outcome.quantity, 25, "{name}");
        let lost = &root.transfers[&transfer];
        assert_eq!(lost.state, ResourceTransferState::Lost, "{name}");
        assert_eq!(
            (lost.escrow, lost.lost, lost.quantity),
            (0, 30, 30),
            "{name}"
        );
        assert_eq!(root.conservation.admitted_loss, 30, "{name}");
        // The transfer's demand was credited nothing.
        assert_eq!(root.demands[&leg.demand].fulfilled, 0, "{name}");
        root.validate().expect("whole loss after a partial loss");
    }
}

/// A plugin-owned run over [`world`] with `records` of evidence and an
/// archive.
fn plugin_run(
    seed: u64,
    records: &[&DomainRecordVersionRef],
    plugins: &[&dyn SimulationPlugin],
) -> Run {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(SOURCE)),
            EntityRef::Person(PersonId::new(DESTINATION)),
        ],
    )
    .with_domain_records(
        records
            .iter()
            .copied()
            .map(evidence_record)
            .chain(world().into_records().expect("resource records"))
            .collect(),
    );
    let archive = MemoryArchive::default();
    let mut canwu = Canwu::new_with_plugins(seed, scenario, plugins).expect("runtime");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    Run {
        canwu,
        archive,
        commands: 0,
    }
}

impl Run {
    /// Activates one single-step lease the host binds to `binding`.
    fn bound_lease(
        &mut self,
        lease_holder: u64,
        suffix: &str,
        operation_key: &ResourceOperationKey,
        targets: &[CompletionLockedTargetV1],
        at: SimTime,
        binding: CompletionOperationBindingV1,
    ) -> CompletionLeaseActivationCertificateV1 {
        let mut lease = lease_request(lease_holder, suffix, operation_key, targets, at);
        lease.eligibility_envelope = lease
            .eligibility_envelope
            .bound_to(binding)
            .expect("bound envelope");
        let operation =
            ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
                lease,
                grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}"))
                    .expect("grant"),
                target_versions: targets.to_vec(),
                current_boundary: next_boundary(&self.canwu),
            });
        enqueue_resource_completion_operation(&mut self.canwu, at, &operation)
            .expect("completion ingress");
        settle(&mut self.canwu, at);
        state(&self.canwu).completion_leases.certificates[&acquisition(suffix)].clone()
    }

    /// The host gives up the single-step lease `suffix`, which unlocks its
    /// targets.
    fn release(&mut self, suffix: &str, at: SimTime) {
        let current = state(&self.canwu);
        let grant =
            CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}")).expect("grant");
        let operation = ResourceCompletionOperationV1::Release(ReleaseCompletionCapacityV1 {
            acquisition: acquisition(suffix),
            expected_acquisition_revision: current.completion_leases.acquisitions
                [&acquisition(suffix)]
                .revision,
            expected_grant_revision: current.completion_leases.grants[&grant].revision,
            grant,
            reason: "fixture: the probe was refused".to_owned(),
        });
        enqueue_resource_completion_operation(&mut self.canwu, at, &operation)
            .expect("completion ingress");
        settle(&mut self.canwu, at);
        assert!(
            !state(&self.canwu)
                .completion_leases
                .target_locks
                .values()
                .any(|holder| holder
                    == &CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}"))
                        .expect("grant"))
        );
    }

    /// Runs an allocation pass of the source custodian's demands at `at` and
    /// returns its outcome.
    fn allocate(&mut self, name: &str, at: SimTime) -> ResourceOperationOutcome {
        let operation_key = key(&format!("fixture:allocate:{name}"));
        let expected_state_revision = state(&self.canwu).state_revision;
        enqueue_resource_allocation(
            &mut self.canwu,
            at,
            &holder(SOURCE),
            &ResourceAllocationRequestV1 {
                operation_key: operation_key.clone(),
                expected_state_revision,
                at,
                candidate_limit: 8,
            },
        )
        .expect("allocation ingress");
        settle(&mut self.canwu, at);
        state(&self.canwu).outcomes[&operation_key].clone()
    }

    /// Submits `demand` and allocates it, then starts a transfer of its leg
    /// to the destination account, all at `at`. Returns the leg.
    fn start(&mut self, name: &str, demand: ResourceDemand, at: SimTime) -> ResourceAllocationLeg {
        self.applied(
            SOURCE,
            at,
            ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
                operation_key: key(&format!("fixture:submit:{name}")),
                demand,
            }),
        );
        let allocation = self.allocate(name, at);
        assert_eq!(allocation.status, ResourceOperationStatus::Applied);
        let current = state(&self.canwu);
        let leg = leg_of(&current, name);
        let start_key = key(&format!("fixture:start:{name}"));
        let start = self.lease(
            SOURCE,
            &format!("start-{name}"),
            &start_key,
            &start_targets(&current, &leg),
            at,
        );
        self.applied(
            SOURCE,
            at,
            ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
                operation_key: start_key,
                transfer_id: ResourceTransferId::new(format!("fixture:transfer:{name}"))
                    .expect("transfer"),
                allocation: (&leg).into(),
                expected_account_revision: current.accounts[&leg.account].revision,
                destination: Some(account("destination")),
                at,
                completion_certificate: start,
            }),
        );
        leg
    }

    /// The source custodian loses `quantity` of the transfer `name` at `at`.
    fn lose_part(
        &mut self,
        name: &str,
        quantity: u64,
        cause: &DomainRecordVersionRef,
        at: SimTime,
    ) {
        let transfer = ResourceTransferId::new(format!("fixture:transfer:{name}")).expect("id");
        let current = state(&self.canwu);
        let loss_key = key(&format!("fixture:loss:{name}"));
        let certificate = self.lease(
            SOURCE,
            &format!("loss-{name}"),
            &loss_key,
            &loss_targets(&current, &transfer, cause),
            at,
        );
        self.applied(
            SOURCE,
            at,
            transfer_loss(
                name,
                &transfer,
                current.transfers[&transfer].revision,
                quantity,
                cause,
                at,
                certificate,
            ),
        );
    }

    /// The destination custodian accepts the transfer `name` locally at `at`.
    fn accept(
        &mut self,
        name: &str,
        handover: &DomainRecordVersionRef,
        at: SimTime,
    ) -> ResourceOperationOutcome {
        let transfer = ResourceTransferId::new(format!("fixture:transfer:{name}")).expect("id");
        let current = state(&self.canwu);
        let transfer_revision = current.transfers[&transfer].revision;
        let destination_revision = current.accounts[&account("destination")].revision;
        let accept_key = key(&format!("fixture:accept:{name}"));
        let certificate = self.lease(
            DESTINATION,
            &format!("accept-{name}"),
            &accept_key,
            &[
                CompletionLockedTargetV1::Transfer {
                    id: transfer.clone(),
                    revision: transfer_revision,
                },
                CompletionLockedTargetV1::Account {
                    id: account("destination"),
                    revision: destination_revision,
                },
                CompletionLockedTargetV1::ExternalRecord {
                    version: handover.clone(),
                },
            ],
            at,
        );
        self.applied(
            DESTINATION,
            at,
            ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
                operation_key: accept_key,
                transfer,
                expected_transfer_revision: transfer_revision,
                at,
                disposition: ResourceTransferDispositionV1::AcceptLocal {
                    destination: account("destination"),
                    expected_destination_revision: destination_revision,
                    handover_evidence: handover.clone(),
                },
                exact_transport_evidence: None,
                completion_certificate: certificate,
            }),
        )
    }
}

#[test]
fn a_demand_that_ends_while_its_transfer_is_in_flight_keeps_its_status_and_counts_the_delivery() {
    let cause = evidence("cause");
    let handover = evidence("handover");
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    for (seed, ending) in [(165, DemandStatus::Cancelled), (265, DemandStatus::Expired)] {
        let name = format!("{ending:?}").to_lowercase();
        let mut run = plugin_run(seed, &[&cause, &handover], &plugins);
        // A transfer of 30 for a demand of 30 that expires at minute two.
        let at = minute(1);
        let mut request = demand(&state(&run.canwu), &name, 30, at);
        request.expires_at = minute(2);
        let leg = run.start(&name, request, at);

        // The demand ends while the transfer is in flight: its requester
        // cancels it, or a pass at its expiry expires it.
        let at = minute(2);
        if ending == DemandStatus::Cancelled {
            let revision = state(&run.canwu).demands[&leg.demand].revision;
            run.applied(
                SOURCE,
                at,
                ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
                    operation_key: key(&format!("fixture:cancel:{name}")),
                    demand: leg.demand.clone(),
                    expected_demand_revision: revision,
                }),
            );
        } else {
            run.allocate(&format!("{name}:expiry"), at);
        }
        assert_eq!(state(&run.canwu).demands[&leg.demand].status, ending);
        // An archive commit leaves the ended demand and its leg in the root
        // while the transfer drawn on them is in flight.
        run.archive_all();
        let archived = state(&run.canwu);
        assert!(archived.demands.contains_key(&leg.demand), "{name}");
        assert!(archived.allocation_legs.contains_key(&leg.id), "{name}");
        assert!(archived.terminal_archive_candidates.values().all(|key| {
            *key != ResourceTerminalRecordKeyV1::Demand(leg.demand.clone())
                && *key != ResourceTerminalRecordKeyV1::AllocationLeg(leg.id.clone())
        }));

        // Ten units are lost and the destination accepts the other twenty;
        // every boundary settles.
        run.lose_part(&name, 10, &cause, at);
        let accepted = run.accept(&name, &handover, minute(3));
        assert_eq!((accepted.quantity, accepted.remainder), (20, 10), "{name}");
        let settled = state(&run.canwu);
        let ended = &settled.demands[&leg.demand];
        assert_eq!(ended.status, ending, "{name}: the demand stays terminal");
        assert_eq!((ended.fulfilled, ended.remainder()), (20, 10), "{name}");
        assert!(!settled.dirty_demands.contains(&leg.demand), "{name}");
        assert!(
            settled
                .demand_due_index
                .values()
                .chain(settled.demand_expiry_index.values())
                .all(|ids| !ids.contains(&leg.demand)),
            "{name}"
        );
        assert_eq!(settled.accounts[&account("destination")].balance, 20);
        settled.validate().expect("root after the delivery");
        settled.validate_conservation().expect("conserved");
        // The transfer has ended, so the demand's closure is an archive
        // candidate now, and the next commit takes it.
        assert!(
            settled
                .terminal_archive_candidates
                .values()
                .any(|key| *key == ResourceTerminalRecordKeyV1::Demand(leg.demand.clone())),
            "{name}"
        );
        // A later pass finds nothing to do for it, and the run goes on.
        let pass = run.allocate(&format!("{name}:later"), minute(4));
        assert_eq!(pass.status, ResourceOperationStatus::Applied);
        assert_eq!(pass.quantity, 0, "{name}");
        run.archive_all();
        let archived = state(&run.canwu);
        assert!(!archived.demands.contains_key(&leg.demand), "{name}");
        assert!(!archived.allocation_legs.contains_key(&leg.id), "{name}");

        let settled = state(&run.canwu);
        let restored = from_resource_snapshot_json_with_archive_store(
            &run.canwu.snapshot_json().expect("snapshot"),
            &plugins,
            Arc::new(run.archive.clone()),
        )
        .expect("checked restore");
        assert_eq!(state(&restored), settled, "{name}");
        let replayed = replay_resource_from_journal_with_archive_store(
            &plugins,
            &run.canwu.replay_journal(),
            Arc::new(run.archive.clone()),
        )
        .expect("exact replay");
        assert_eq!(state(&replayed), settled, "{name}");
    }
}

#[test]
fn an_allocation_pass_counts_what_is_in_flight_and_a_loss_reopens_only_what_was_lost() {
    let cause = evidence("cause");
    let handover = evidence("handover");
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut run = plugin_run(365, &[&cause, &handover], &plugins);

    // A transfer of 60 for a demand of 60 that is useful only whole; 40 stay
    // in the source account.
    let at = minute(1);
    let mut request = demand(&state(&run.canwu), "flight", 60, at);
    request.minimum_useful = 60;
    let leg = run.start("flight", request, at);
    assert_eq!(leg.quantity, 60);
    let legs = |run: &Run| {
        state(&run.canwu)
            .allocation_legs
            .values()
            .filter(|value| value.demand == leg.demand)
            .count()
    };

    // A pass while the 60 are in flight allocates nothing more, and does not
    // reject the demand for its minimum though the 40 left could not meet it.
    let pass = run.allocate("in-flight", minute(2));
    assert_eq!(pass.status, ResourceOperationStatus::Applied);
    assert_eq!(pass.quantity, 0);
    assert_eq!(legs(&run), 1);
    let flying = state(&run.canwu);
    assert_eq!(flying.demands[&leg.demand].status, DemandStatus::Open);
    assert_eq!(flying.accounts[&account("source")].balance, 40);

    // Ten are lost: the next pass allocates those ten, and only those.
    run.lose_part("flight", 10, &cause, minute(2));
    let pass = run.allocate("after-loss", minute(3));
    assert_eq!(pass.quantity, 10);
    assert_eq!(legs(&run), 2);
    let reopened = state(&run.canwu);
    let second = reopened
        .allocation_legs
        .values()
        .find(|value| value.demand == leg.demand && value.id != leg.id)
        .expect("second leg");
    assert_eq!(second.quantity, 10);

    // The other fifty are accepted: the demand holds fifty fulfilled and ten
    // reserved, and a further pass allocates nothing.
    let accepted = run.accept("flight", &handover, minute(3));
    assert_eq!(accepted.quantity, 50);
    let delivered = state(&run.canwu);
    let flight = &delivered.demands[&leg.demand];
    assert_eq!(flight.status, DemandStatus::PartiallyFulfilled);
    assert_eq!((flight.fulfilled, flight.remainder()), (50, 10));
    let pass = run.allocate("after-acceptance", minute(4));
    assert_eq!(pass.quantity, 0);
    assert_eq!(legs(&run), 2);
    let settled = state(&run.canwu);
    settled.validate().expect("root after the passes");
    settled.validate_conservation().expect("conserved");

    let restored =
        from_resource_snapshot_json(&run.canwu.snapshot_json().expect("snapshot"), &plugins)
            .expect("checked restore");
    assert_eq!(state(&restored), settled);
    let replayed =
        replay_resource_from_journal(&plugins, &run.canwu.replay_journal()).expect("exact replay");
    assert_eq!(state(&replayed), settled);
}

#[test]
fn a_returning_transfer_does_not_hold_back_its_demand() {
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut run = plugin_run(565, &[], &plugins);
    // A transfer of 30 for a demand of 30; 70 stay in the source account.
    let at = minute(1);
    let request = demand(&state(&run.canwu), "returning", 30, at);
    let leg = run.start("returning", request, at);
    let pass = run.allocate("in-flight", at);
    assert_eq!(pass.quantity, 0);

    // Cancelled, the transfer is returning and can no longer be accepted, so
    // its escrow no longer counts for the demand: a pass allocates 30 again.
    let transfer = ResourceTransferId::new("fixture:transfer:returning").expect("id");
    let revision = state(&run.canwu).transfers[&transfer].revision;
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::CancelTransfer(ResourceTransferCancellationRequestV1 {
            operation_key: key("fixture:cancel:returning"),
            transfer: transfer.clone(),
            expected_transfer_revision: revision,
            at,
        }),
    );
    assert_eq!(
        state(&run.canwu).transfers[&transfer].state,
        ResourceTransferState::ReturnPending
    );
    let pass = run.allocate("after-cancel", minute(2));
    assert_eq!(pass.quantity, 30);
    let reallocated = state(&run.canwu);
    assert_eq!(
        reallocated
            .allocation_legs
            .values()
            .filter(|value| value.demand == leg.demand)
            .count(),
        2
    );
    assert_eq!(
        reallocated
            .account_quantities(&account("source"))
            .expect("quantities")
            .reserved,
        30
    );
    reallocated
        .validate()
        .expect("root after the second allocation");
}

#[test]
fn a_lease_bound_to_one_escrow_or_disposition_is_not_redeemed_for_another() {
    let cause = evidence("cause");
    let handover = evidence("handover");
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut run = plugin_run(665, &[&cause, &handover], &plugins);
    // A transfer of 30 that loses 10.
    let at = minute(1);
    let request = demand(&state(&run.canwu), "bound", 30, at);
    run.start("bound", request, at);
    run.lose_part("bound", 10, &cause, minute(2));
    let transfer = ResourceTransferId::new("fixture:transfer:bound").expect("id");

    // A lease the host bound to an acceptance of the 30 started is not
    // redeemed for the 20 left. Only the settlement knows the escrow, so the
    // acceptance is its recorded rejection.
    let at = minute(3);
    let current = state(&run.canwu);
    let transfer_revision = current.transfers[&transfer].revision;
    let destination_revision = current.accounts[&account("destination")].revision;
    let accept_key = key("fixture:accept:bound");
    let certificate = run.bound_lease(
        DESTINATION,
        "accept-bound",
        &accept_key,
        &[
            CompletionLockedTargetV1::Transfer {
                id: transfer.clone(),
                revision: transfer_revision,
            },
            CompletionLockedTargetV1::Account {
                id: account("destination"),
                revision: destination_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: handover.clone(),
            },
        ],
        at,
        CompletionOperationBindingV1::new(ResourceOperationKind::CompleteTransfer, 30),
    );
    let outcome = run.settled(
        DESTINATION,
        at,
        ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
            operation_key: accept_key,
            transfer: transfer.clone(),
            expected_transfer_revision: transfer_revision,
            at,
            disposition: ResourceTransferDispositionV1::AcceptLocal {
                destination: account("destination"),
                expected_destination_revision: destination_revision,
                handover_evidence: handover.clone(),
            },
            exact_transport_evidence: None,
            completion_certificate: certificate,
        }),
    );
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_authority"));
    assert_eq!(state(&run.canwu).transfers[&transfer].escrow, 20);
    run.release("accept-bound", at);

    // A lease bound to losing the 20 left is not redeemed for their return:
    // the request names its disposition, so the tracked return is refused at
    // admission and the loss then settles under the lease's key.
    let lose_key = key("fixture:lose:bound");
    let current = state(&run.canwu);
    let certificate = run.bound_lease(
        SOURCE,
        "lose-bound",
        &lose_key,
        &loss_targets(&current, &transfer, &cause),
        at,
        CompletionOperationBindingV1::new(ResourceOperationKind::CompleteTransfer, 20)
            .with_disposition(ResourceTransferDispositionKindV1::Lose),
    );
    let disposition = |disposition| {
        ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
            operation_key: lose_key.clone(),
            transfer: transfer.clone(),
            expected_transfer_revision: current.transfers[&transfer].revision,
            at,
            disposition,
            exact_transport_evidence: None,
            completion_certificate: certificate.clone(),
        })
    };
    let attempt = run.command(
        SOURCE,
        at,
        disposition(ResourceTransferDispositionV1::Return {
            expected_source_revision: current.accounts[&account("source")].revision,
        }),
    );
    let CommandAttemptOutcome::Rejected { error } = attempt else {
        panic!("a return under a lease bound to a loss was admitted: {attempt:?}");
    };
    assert!(
        error.message.contains("bound to another operation"),
        "{error:?}"
    );
    assert!(!state(&run.canwu).outcomes.contains_key(&lose_key));
    run.applied(
        SOURCE,
        at,
        disposition(ResourceTransferDispositionV1::Lose {
            loss_id: ResourceLossId::new("fixture:lose:bound").expect("loss"),
            cause: EvidenceRef::DomainRecordVersion(cause.clone()),
        }),
    );
    let lost = state(&run.canwu);
    let done = &lost.transfers[&transfer];
    assert_eq!(done.state, ResourceTransferState::Lost);
    assert_eq!((done.lost, done.escrow), (30, 0));
    lost.validate().expect("root after the loss");
    lost.validate_conservation().expect("conserved");
}

#[test]
fn a_demand_cancelled_in_flight_is_archived_once_its_transfer_is_returned() {
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut run = plugin_run(765, &[], &plugins);
    let at = minute(1);
    let request = demand(&state(&run.canwu), "recalled", 30, at);
    let leg = run.start("recalled", request, at);
    let transfer = ResourceTransferId::new("fixture:transfer:recalled").expect("id");
    // The demand is cancelled and its transfer called back.
    let current = state(&run.canwu);
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:cancel-demand:recalled"),
            demand: leg.demand.clone(),
            expected_demand_revision: current.demands[&leg.demand].revision,
        }),
    );
    let revision = state(&run.canwu).transfers[&transfer].revision;
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::CancelTransfer(ResourceTransferCancellationRequestV1 {
            operation_key: key("fixture:cancel:recalled"),
            transfer: transfer.clone(),
            expected_transfer_revision: revision,
            at,
        }),
    );
    run.archive_all();
    assert!(state(&run.canwu).demands.contains_key(&leg.demand));

    // The return ends the transfer; the demand's closure follows it into
    // the archive.
    let at = minute(2);
    let current = state(&run.canwu);
    let source_revision = current.accounts[&account("source")].revision;
    let return_key = key("fixture:return:recalled");
    let certificate = run.lease(
        SOURCE,
        "return-recalled",
        &return_key,
        &[
            CompletionLockedTargetV1::Transfer {
                id: transfer.clone(),
                revision: current.transfers[&transfer].revision,
            },
            CompletionLockedTargetV1::Account {
                id: account("source"),
                revision: source_revision,
            },
        ],
        at,
    );
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
            operation_key: return_key,
            transfer: transfer.clone(),
            expected_transfer_revision: current.transfers[&transfer].revision,
            at,
            disposition: ResourceTransferDispositionV1::Return {
                expected_source_revision: source_revision,
            },
            exact_transport_evidence: None,
            completion_certificate: certificate,
        }),
    );
    let returned = state(&run.canwu);
    assert_eq!(returned.accounts[&account("source")].balance, 100);
    assert!(
        returned
            .terminal_archive_candidates
            .values()
            .any(|key| *key == ResourceTerminalRecordKeyV1::Demand(leg.demand.clone()))
    );
    run.archive_all();
    let archived = state(&run.canwu);
    assert!(!archived.demands.contains_key(&leg.demand));
    assert!(!archived.allocation_legs.contains_key(&leg.id));
    archived.validate().expect("root after the archive");
}

#[test]
fn a_root_with_an_ended_demand_neither_archived_nor_held_is_refused() {
    let mut root = world();
    let demand = demand(&root, "leaked", 30, SimTime::EPOCH);
    let id = demand.id.clone();
    root.install_demand(demand).expect("demand");
    root.validate().expect("an open demand");
    // An ended demand that no transfer in flight holds and that is not an
    // archive candidate is a closure that leaked.
    root.demands.get_mut(&id).expect("demand").status = DemandStatus::Cancelled;
    let error = root.validate().expect_err("a leaked closure");
    assert!(
        error
            .to_string()
            .contains("neither an archive candidate nor held back"),
        "{error}"
    );
}

#[test]
fn an_amendment_asks_for_no_less_than_is_fulfilled_reserved_and_in_flight() {
    let handover = evidence("handover");
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut run = plugin_run(865, &[&handover], &plugins);

    // A demand of 150 on two of the requester's accounts gets legs of 100
    // and 50; the 100 leave in a transfer.
    let at = minute(1);
    let mut request = demand(&state(&run.canwu), "amended", 150, at);
    request.source_policy =
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![account("source"), account("spare")]);
    let demand_id = request.id.clone();
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
            operation_key: key("fixture:submit:amended"),
            demand: request,
        }),
    );
    assert_eq!(run.allocate("amended", at).quantity, 150);
    let current = state(&run.canwu);
    let leg = current
        .allocation_legs
        .values()
        .find(|value| value.demand == demand_id && value.quantity == 100)
        .cloned()
        .expect("leg of 100");
    let start_key = key("fixture:start:amended");
    let start = run.lease(
        SOURCE,
        "start-amended",
        &start_key,
        &start_targets(&current, &leg),
        at,
    );
    run.applied(
        SOURCE,
        at,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: ResourceTransferId::new("fixture:transfer:amended").expect("transfer"),
            allocation: (&leg).into(),
            expected_account_revision: current.accounts[&leg.account].revision,
            destination: Some(account("destination")),
            at,
            completion_certificate: start,
        }),
    );

    // Asking for 100 or 149 while 50 are reserved and 100 in flight is a
    // recorded rejection.
    let amend = |run: &Run, name: &str, requested: u64| {
        let current = state(&run.canwu).demands[&demand_id].clone();
        let mut replacement = current.clone();
        replacement.requested = requested;
        ResourceOperationRequestV1::AmendDemand(ResourceAmendDemandRequestV1 {
            operation_key: key(&format!("fixture:amend:{name}")),
            replacement,
            expected_demand_revision: current.revision,
        })
    };
    for (name, requested) in [("down", 100), ("short", 149)] {
        let request = amend(&run, name, requested);
        let outcome = run.settled(SOURCE, at, request);
        assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{name}");
        assert_eq!(
            outcome.rejection_code.as_deref(),
            Some("invalid_lifecycle"),
            "{:?}",
            outcome.rejection_reason
        );
        assert_eq!(state(&run.canwu).demands[&demand_id].requested, 150);
    }
    // Within bounds, the amendment settles.
    let request = amend(&run, "up", 160);
    run.applied(SOURCE, at, request);

    // The destination accepts the 100, the requester cancels the rest, and
    // an archive commit takes the demand's closure; no boundary fails.
    let accepted = run.accept("amended", &handover, minute(2));
    assert_eq!(accepted.quantity, 100);
    let partial = state(&run.canwu);
    assert_eq!(
        partial.demands[&demand_id].status,
        DemandStatus::PartiallyFulfilled
    );
    run.applied(
        SOURCE,
        minute(2),
        ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:cancel:amended"),
            demand: demand_id.clone(),
            expected_demand_revision: partial.demands[&demand_id].revision,
        }),
    );
    run.archive_all();
    let archived = state(&run.canwu);
    assert!(!archived.demands.contains_key(&demand_id));
    assert_eq!(archived.accounts[&account("spare")].balance, 50);
    assert_eq!(
        archived
            .account_quantities(&account("spare"))
            .expect("quantities")
            .reserved,
        0
    );
    archived.validate().expect("root after the archive");
    archived.validate_conservation().expect("conserved");
}
