//! Downstream gap G-63: a transfer carried by a multi-leg execution records
//! each intermediate leg and handoff, in order, and keeps them into the
//! archive.
//!
//! In a plugin-owned run, a provider advances a transfer through its
//! dispatch, intermediate legs, and a handoff. An intermediate advance that
//! names nothing new, a lower leg, or another execution is a recorded
//! `invalid_lifecycle` rejection, and one past the fourteen in-transit links
//! a transfer records is a recorded `limit_exceeded` rejection; no boundary
//! fails. The arrival and the return are still admitted at the cap, the
//! transfer is returned under a tracked command, and its archived record names
//! all sixteen links in the order they were travelled. A restored run and an
//! exact replay agree.
//!
//! An archived transfer whose history its advances could not have recorded,
//! whose quantity its dispositions do not account for, or that was accepted
//! without an arrival, is refused.
//!
//! A second fixture keeps an open transfer's transport evidence in its
//! history only, and refuses a live root whose open transfer was forged (a
//! lower leg, a seventeenth link, a current link that is not the last, no
//! link while in transit, evidence kept twice) when it is restored from its
//! records and when a run starts from it.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, HandoffId, Issuer,
    ItineraryRevisionId, KnowledgeHolderRef, LegExecutionId, PayloadSchema, PersonId,
    PluginArchiveObjectProvider, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
    TransportExecutionId,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

const NAMESPACE: &str = "fixture.resource";
const EVIDENCE_NAMESPACE: &str = "fixture.provider";
const PROVIDER: &str = "fixture-provider";
const SOURCE: u64 = 1;
const DESTINATION: u64 = 2;
/// Transport records the provider cites, one per advance.
const TRANSPORT_RECORDS: usize = 24;

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

fn transport_record(index: usize) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, "transport", format!("leg-{index}")),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn evidence_record(version: &DomainRecordVersionRef) -> DomainRecord {
    DomainRecord {
        reference: version.record.clone(),
        owner: PROVIDER.to_owned(),
        class: DomainRecordClass::Record,
        version: version.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "transport": version.record.id }),
        references: Vec::new(),
    }
}

/// The provider's transport records.
struct TransportRecord;

impl DomainRecordType for TransportRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "transport";
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
        "0000000000000000000000000000000000000000000000000000000000000063"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<TransportRecord>();
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

/// A source account of 100 with a demand of 30 on it, and an empty
/// destination account.
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
    for (name, custodian, balance) in [("source", SOURCE, 100), ("destination", DESTINATION, 0)] {
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
                place_scope: None,
            })
            .expect("account");
    }
    state
        .install_demand(ResourceDemand {
            id: ResourceDemandId::new("fixture:demand:cargo").expect("demand"),
            revision: ResourceRevision::INITIAL,
            requester: holder(SOURCE),
            source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![account("source")]),
            resource_revision: resource,
            unit_revision: unit,
            requested: 30,
            fulfilled: 0,
            minimum_useful: 30,
            partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
            alternative_group: None,
            due_at: SimTime::EPOCH,
            expires_at: SimTime::EPOCH + SimDuration::days(10),
            priority: 1,
            tie_break: ResourceTieBreakKey::new("fixture:tie:cargo").expect("tie"),
            admitted_sequence: 0,
            protected_floor_policy: None,
            protection_override_class: None,
            status: DemandStatus::Open,
            rejection_reason: None,
        })
        .expect("demand");
    let revision = state.state_revision;
    state
        .apply_operation(&ResourceOperationRequestV1::Allocate(
            ResourceAllocationRequestV1 {
                operation_key: key("fixture:allocate:cargo"),
                expected_state_revision: revision,
                at: SimTime::EPOCH,
                candidate_limit: 8,
            },
        ))
        .expect("allocation");
    state
}

/// A link of execution `execution`, leg `leg`, after handoff `handoff`.
fn link(execution: u64, leg: Option<u64>, handoff: Option<u64>) -> TransportExecutionLink {
    TransportExecutionLink {
        execution: TransportExecutionId(execution),
        itinerary_revision: ItineraryRevisionId(1),
        leg_execution: leg.map(LegExecutionId),
        handoff: handoff.map(HandoffId),
        capacity_booking: None,
    }
}

fn single_step(
    lease_holder: u64,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: &[CompletionLockedTargetV1],
    at: SimTime,
    current_boundary: u64,
) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
        lease: RequestCompletionLeaseV1 {
            id: acquisition(suffix),
            operation_key: operation_key.clone(),
            holder: holder(lease_holder),
            operation_namespace: NAMESPACE.to_owned(),
            eligibility_time: at,
            eligibility_envelope: EligibilityEnvelopeV1::new(
                Vec::new(),
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
        },
        grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}")).expect("grant"),
        target_versions: targets.to_vec(),
        current_boundary,
    })
}

/// A change that makes an archived transfer one its operations could not
/// have left.
type Forgery = fn(&mut ResourceTransfer);

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

/// A plugin-owned run whose provider advances one transfer.
struct Run {
    canwu: Canwu,
    transfer: ResourceTransferId,
    commands: u64,
    advances: usize,
}

impl Run {
    fn command(&mut self, actor: u64, at: SimTime, request: ResourceOperationRequestV1) {
        let operation_key = request.operation_key();
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
        assert!(matches!(
            self.canwu
                .command_attempts()
                .last()
                .expect("command attempt")
                .outcome,
            CommandAttemptOutcome::Accepted { .. }
        ));
        settle(&mut self.canwu, at);
        let outcome = &state(&self.canwu).outcomes[&operation_key];
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{:?}",
            outcome.rejection_reason
        );
    }

    fn lease(
        &mut self,
        lease_holder: u64,
        suffix: &str,
        operation_key: &ResourceOperationKey,
        targets: &[CompletionLockedTargetV1],
        at: SimTime,
    ) -> CompletionLeaseActivationCertificateV1 {
        let operation = single_step(
            lease_holder,
            suffix,
            operation_key,
            targets,
            at,
            next_boundary(&self.canwu),
        );
        enqueue_resource_completion_operation(&mut self.canwu, at, &operation)
            .expect("completion ingress");
        settle(&mut self.canwu, at);
        state(&self.canwu).completion_leases.certificates[&acquisition(suffix)].clone()
    }

    /// The provider advances the transfer to `progress` on `transport`, in a
    /// boundary of its own, citing a transport record of its own. Returns
    /// the advance's recorded outcome.
    fn advance(
        &mut self,
        progress: TransferProgressV1,
        transport: TransportExecutionLink,
    ) -> ResourceOperationOutcome {
        let at = minute(1);
        let evidence = transport_record(self.advances);
        let operation_key = key(&format!("fixture:advance:{}", self.advances));
        self.advances += 1;
        let packet = ResourceAdapterOperationV1 {
            provider_plugin: PROVIDER.to_owned(),
            provider_source: evidence.clone(),
            request: ResourceOperationRequestV1::AdvanceTransfer(
                ResourceTransferProgressRequestV1 {
                    operation_key: operation_key.clone(),
                    transfer: self.transfer.clone(),
                    expected_transfer_revision: state(&self.canwu).transfers[&self.transfer]
                        .revision,
                    progress,
                    transport,
                    transport_evidence: evidence,
                },
            ),
        };
        enqueue_resource_adapter_operation(&mut self.canwu, at, &packet).expect("adapter ingress");
        settle(&mut self.canwu, at);
        state(&self.canwu).outcomes[&operation_key].clone()
    }

    /// An advance that is the recorded rejection `code` and changes nothing
    /// of the transfer.
    fn refused(&mut self, transport: TransportExecutionLink, code: &str) {
        let before = state(&self.canwu).transfers[&self.transfer].clone();
        let outcome = self.advance(TransferProgressV1::InTransit, transport);
        assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{code}");
        assert_eq!(
            outcome.rejection_code.as_deref(),
            Some(code),
            "{:?}",
            outcome.rejection_reason
        );
        assert_eq!(state(&self.canwu).transfers[&self.transfer], before);
    }

    fn applied(&mut self, progress: TransferProgressV1, transport: TransportExecutionLink) {
        let outcome = self.advance(progress, transport);
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{:?}",
            outcome.rejection_reason
        );
    }
}

/// A plugin-owned run in which the source custodian has started a transfer
/// of 30 from the source to the destination account.
fn started(plugins: &[&dyn SimulationPlugin], archive: Option<&MemoryArchive>) -> Run {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(SOURCE)),
            EntityRef::Person(PersonId::new(DESTINATION)),
        ],
    )
    .with_domain_records(
        (0..TRANSPORT_RECORDS)
            .map(|index| evidence_record(&transport_record(index)))
            .chain(world().into_records().expect("resource records"))
            .collect(),
    );
    let mut canwu = Canwu::new_with_plugins(63, scenario, plugins).expect("runtime");
    if let Some(archive) = archive {
        canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    }
    let transfer = ResourceTransferId::new("fixture:transfer:cargo").expect("transfer");
    let mut run = Run {
        canwu,
        transfer: transfer.clone(),
        commands: 0,
        advances: 0,
    };
    let at = minute(1);
    let current = state(&run.canwu);
    let leg = current
        .allocation_legs
        .values()
        .next()
        .cloned()
        .expect("allocation leg");
    let start_key = key("fixture:start:cargo");
    let start = run.lease(
        SOURCE,
        "start",
        &start_key,
        &[
            CompletionLockedTargetV1::Account {
                id: leg.account.clone(),
                revision: current.accounts[&leg.account].revision,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: leg.id.clone(),
                revision: leg.revision,
            },
            CompletionLockedTargetV1::Demand {
                id: leg.demand.clone(),
                revision: leg.demand_revision,
            },
        ],
        at,
    );
    run.command(
        SOURCE,
        at,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: transfer,
            allocation: (&leg).into(),
            expected_account_revision: current.accounts[&leg.account].revision,
            destination: Some(account("destination")),
            at,
            completion_certificate: start,
        }),
    );
    run
}

#[test]
fn a_transfer_records_its_legs_and_handoffs_in_order_and_archives_them() {
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "transport")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let archive = MemoryArchive::default();
    // The source custodian starts a transfer of 30.
    let mut run = started(&plugins, Some(&archive));
    let transfer = run.transfer.clone();

    // Dispatch, a second leg, and a handoff within that leg.
    run.applied(TransferProgressV1::InTransit, link(7, Some(1), None));
    run.applied(TransferProgressV1::InTransit, link(7, Some(2), None));
    run.applied(TransferProgressV1::InTransit, link(7, Some(2), Some(1)));
    // Nothing new, a lower leg, another execution, and a link without a leg
    // after one with a leg: each a recorded rejection.
    run.refused(link(7, Some(2), Some(1)), "invalid_lifecycle");
    run.refused(link(7, Some(1), Some(2)), "invalid_lifecycle");
    run.refused(link(8, Some(3), None), "invalid_lifecycle");
    run.refused(link(7, None, Some(3)), "invalid_lifecycle");
    // Eleven more legs reach the fourteen in-transit links a transfer
    // records; the next is refused, the arrival and the return are not.
    for leg in 3..=13 {
        run.applied(TransferProgressV1::InTransit, link(7, Some(leg), None));
    }
    run.refused(link(7, Some(14), None), "limit_exceeded");
    run.applied(
        TransferProgressV1::ArrivalPending,
        link(7, Some(13), Some(2)),
    );
    run.applied(
        TransferProgressV1::ReturnPending,
        link(7, Some(13), Some(3)),
    );
    let returning = state(&run.canwu).transfers[&transfer].clone();
    assert_eq!(returning.state, ResourceTransferState::ReturnPending);
    assert_eq!(returning.link_history.len(), MAX_RESOURCE_TRANSFER_LINKS);
    assert_eq!(
        returning.transport.as_ref(),
        returning.link_history.last().map(|entry| &entry.transport)
    );
    let progress: Vec<_> = returning
        .link_history
        .iter()
        .map(|entry| entry.progress)
        .collect();
    assert_eq!(
        progress,
        std::iter::repeat_n(TransferProgressV1::InTransit, 14)
            .chain([
                TransferProgressV1::ArrivalPending,
                TransferProgressV1::ReturnPending
            ])
            .collect::<Vec<_>>()
    );
    let legs: Vec<_> = returning
        .link_history
        .iter()
        .map(|entry| entry.transport.leg_execution.map(|leg| leg.0))
        .collect();
    assert_eq!(
        legs,
        [1, 2, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 13, 13]
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>()
    );
    // Each entry keeps the evidence its advance cited, in order: the
    // refused advances cited the records between.
    let cited: Vec<_> = returning
        .link_history
        .iter()
        .map(|entry| entry.evidence.clone())
        .collect();
    let applied: Vec<_> = [0, 1, 2]
        .into_iter()
        .chain(7..=17)
        .chain([19, 20])
        .map(transport_record)
        .collect();
    assert_eq!(cited, applied);

    // The return completes at the cap, and the history stays with the
    // terminal transfer.
    let at = minute(2);
    let current = state(&run.canwu);
    let source_revision = current.accounts[&account("source")].revision;
    let return_key = key("fixture:return:cargo");
    let certificate = run.lease(
        SOURCE,
        "return",
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
    run.command(
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
    let returned = state(&run.canwu).transfers[&transfer].clone();
    assert_eq!(returned.state, ResourceTransferState::Returned);
    assert_eq!(returned.link_history, returning.link_history);
    assert_eq!(state(&run.canwu).accounts[&account("source")].balance, 100);

    // An archived transfer whose history or quantities were forged is
    // refused.
    let hot = state(&run.canwu);
    let prepared = hot
        .prepare_resource_archive(
            hot.terminal_archive_candidates.len(),
            &MemoryArchive::default(),
        )
        .expect("prepare archive commit");
    prepared.validate().expect("the archived history is valid");
    let position = prepared
        .blob
        .records
        .iter()
        .position(|record| record.key == ResourceTerminalRecordKeyV1::Transfer(transfer.clone()))
        .expect("the returned transfer is a candidate");
    let forgeries: [(&str, Forgery); 6] = [
        ("legs out of order", |value| value.link_history.swap(4, 5)),
        ("the current link dropped", |value| {
            value.link_history.pop();
        }),
        ("a seventeenth link", |value| {
            let extra = value.link_history[3].clone();
            value.link_history.insert(3, extra);
        }),
        ("an unaccounted quantity", |value| value.returned -= 1),
        // A transfer is accepted at its destination after an arrival, or
        // locally before any link.
        ("accepted after a return", |value| {
            value.state = ResourceTransferState::Accepted;
            value.accepted = value.returned;
            value.returned = 0;
        }),
        ("accepted in transit", |value| {
            value.state = ResourceTransferState::Accepted;
            value.accepted = value.returned;
            value.returned = 0;
            value.link_history.truncate(3);
            value.transport = value
                .link_history
                .last()
                .map(|entry| entry.transport.clone());
        }),
    ];
    for (name, forge) in forgeries {
        let mut forged = prepared.clone();
        let record = &mut forged.blob.records[position];
        let ResourceTerminalArchivePayloadV1::Transfer(value) = &mut record.payload else {
            unreachable!("the record is a transfer");
        };
        forge(value);
        record.semantic_digest =
            canonical_digest("canwu.resource.transfer.v1", value).expect("digest");
        let error = forged.validate().expect_err(name);
        assert!(
            error
                .to_string()
                .contains("archived resource transfer closure is forged"),
            "{name}: {error}"
        );
    }

    // The archive keeps every link, in order.
    let hot = state(&run.canwu);
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), &archive)
        .expect("prepare archive commit");
    let receipt = enqueue_resource_archive(&mut run.canwu, &prepared, &archive).expect("commit");
    settle(&mut run.canwu, at);
    finalize_resource_archive_retention(&mut run.canwu, &archive, &receipt).expect("finalize");
    settle(&mut run.canwu, at);
    let archived = state(&run.canwu);
    assert!(!archived.transfers.contains_key(&transfer));
    let record = archived_resource_terminal_record(
        &archived,
        &archive,
        &ResourceTerminalRecordKeyV1::Transfer(transfer.clone()),
    )
    .expect("archive lookup")
    .expect("archived transfer");
    let ResourceTerminalArchivePayloadV1::Transfer(value) = record.payload else {
        unreachable!("the record is a transfer");
    };
    assert_eq!(value.link_history, returning.link_history);

    // A restored run and an exact replay hold the same root.
    let restored = from_resource_snapshot_json_with_archive_store(
        &run.canwu.snapshot_json().expect("snapshot"),
        &plugins,
        Arc::new(archive.clone()),
    )
    .expect("checked restore");
    assert_eq!(state(&restored), archived);
    let replayed = replay_resource_from_journal_with_archive_store(
        &plugins,
        &run.canwu.replay_journal(),
        Arc::new(archive),
    )
    .expect("exact replay");
    assert_eq!(state(&replayed), archived);
}

/// Applies `forge` to every copy of the transfer `current` that `value`
/// holds, and returns how many copies it changed.
fn forge_transfer(
    value: &mut serde_json::Value,
    current: &ResourceTransfer,
    forge: Forgery,
) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("id").and_then(serde_json::Value::as_str) == Some(current.id.as_str())
                && map.contains_key("allocation_leg")
                && map.contains_key("escrow")
            {
                let mut transfer: ResourceTransfer =
                    serde_json::from_value(serde_json::Value::Object(map.clone()))
                        .expect("transfer");
                if transfer != *current {
                    return 0;
                }
                forge(&mut transfer);
                let serde_json::Value::Object(forged) =
                    serde_json::to_value(&transfer).expect("forged transfer")
                else {
                    unreachable!("a transfer encodes as an object");
                };
                *map = forged;
                return 1;
            }
            map.values_mut()
                .map(|value| forge_transfer(value, current, forge))
                .sum()
        }
        serde_json::Value::Array(values) => values
            .iter_mut()
            .map(|value| forge_transfer(value, current, forge))
            .sum(),
        _ => 0,
    }
}

#[test]
fn a_root_whose_open_transfer_has_a_history_its_advances_could_not_record_is_refused() {
    // A root whose transfer of 30 is in transit on its third link: a
    // dispatch, a second leg, and a handoff within it.
    let mut root = world();
    let transfer = ResourceTransferId::new("fixture:transfer:cargo").expect("transfer");
    let leg = root
        .allocation_legs
        .values()
        .next()
        .cloned()
        .expect("allocation leg");
    let at = minute(1);
    let start_key = key("fixture:start:cargo");
    let source_revision = root.accounts[&leg.account].revision;
    let targets = [
        CompletionLockedTargetV1::Account {
            id: leg.account.clone(),
            revision: source_revision,
        },
        CompletionLockedTargetV1::AllocationLeg {
            id: leg.id.clone(),
            revision: leg.revision,
        },
        CompletionLockedTargetV1::Demand {
            id: leg.demand.clone(),
            revision: leg.demand_revision,
        },
    ];
    let apply = |root: &mut ResourceState, request: ResourceOperationRequestV1| {
        let outcome = root.apply_operation(&request).expect("operation");
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{:?}",
            outcome.rejection_reason
        );
    };
    apply(
        &mut root,
        ResourceOperationRequestV1::Completion(single_step(
            SOURCE, "start", &start_key, &targets, at, 1,
        )),
    );
    let certificate = root.completion_leases.certificates[&acquisition("start")].clone();
    apply(
        &mut root,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: transfer.clone(),
            allocation: (&leg).into(),
            expected_account_revision: source_revision,
            destination: Some(account("destination")),
            at,
            completion_certificate: certificate,
        }),
    );
    for (index, transport) in [
        link(7, Some(1), None),
        link(7, Some(2), None),
        link(7, Some(2), Some(1)),
    ]
    .into_iter()
    .enumerate()
    {
        let expected_transfer_revision = root.transfers[&transfer].revision;
        apply(
            &mut root,
            ResourceOperationRequestV1::AdvanceTransfer(ResourceTransferProgressRequestV1 {
                operation_key: key(&format!("fixture:advance:{index}")),
                transfer: transfer.clone(),
                expected_transfer_revision,
                progress: TransferProgressV1::InTransit,
                transport,
                transport_evidence: transport_record(index),
            }),
        );
    }
    let open = root.transfers[&transfer].clone();
    let open_root = root.clone();
    assert_eq!(open.state, ResourceTransferState::InTransit);
    assert_eq!(open.link_history.len(), 3);
    // The open transfer's transport evidence is its history's, kept once,
    // and a payload-required dependency of the root.
    assert!(open.exact_evidence.is_empty());
    assert_eq!(
        root.computed_continuation(),
        canwu_api::PayloadRequiredEvidenceContinuationV1::completed(),
        "initial-scenario versions need no declaration"
    );
    // A version a boundary established is declared from the history.
    let mut later = root.clone();
    let established = DomainRecordVersionRef {
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: canwu_api::BoundaryId::new(9),
            change_index: 0,
        },
        ..transport_record(1)
    };
    later
        .transfers
        .get_mut(&transfer)
        .expect("open transfer")
        .link_history[1]
        .evidence = established.clone();
    assert_eq!(
        later.computed_continuation(),
        canwu_api::PayloadRequiredEvidenceContinuationV1::active(vec![
            canwu_api::EvidenceRef::DomainRecordVersion(established)
        ])
    );

    // The live root restores untouched, and a run starts from it.
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "transport")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let records = root.into_records().expect("resource records");
    let live = records
        .iter()
        .position(|record| record.reference.id == "resource:runtime:v1")
        .expect("live root record");
    let restore = |records: &[DomainRecord]| {
        ResourceState::from_records(
            &records[live],
            records
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != live)
                .map(|(_, record)| record),
        )
    };
    let start = |records: Vec<DomainRecord>| {
        let scenario = Scenario::new(
            SimTime::EPOCH,
            vec![
                EntityRef::Person(PersonId::new(SOURCE)),
                EntityRef::Person(PersonId::new(DESTINATION)),
            ],
        )
        .with_domain_records(
            (0..TRANSPORT_RECORDS)
                .map(|index| evidence_record(&transport_record(index)))
                .chain(records)
                .collect(),
        );
        Canwu::new_with_plugins(163, scenario, &plugins)
    };
    assert_eq!(
        restore(&records).expect("the untouched root restores"),
        open_root
    );
    start(records.clone()).expect("a run starts from the untouched root");

    // A live root whose open transfer was forged is refused when it is
    // restored from its records, and a run does not start from it. (A
    // snapshot or journal of a run hashes its records, so the run refuses a
    // forged one before the resource plugin reads it.)
    let forgeries: [(&str, Forgery); 5] = [
        ("a lower leg", |value| {
            value.link_history[1].transport = link(7, Some(0), None);
        }),
        ("a seventeenth link", |value| {
            while value.link_history.len() <= MAX_RESOURCE_TRANSFER_LINKS {
                let last = value.link_history[value.link_history.len() - 1].clone();
                value.link_history.push(last);
            }
        }),
        ("a transport that is not the last link", |value| {
            value.transport = Some(link(7, Some(5), None));
        }),
        ("in transit without a link", |value| {
            value.link_history.clear();
            value.transport = None;
        }),
        ("transport evidence kept twice", |value| {
            let evidence = value.link_history[0].evidence.clone();
            value.exact_evidence.push(evidence);
        }),
    ];
    for (name, forge) in forgeries {
        let mut forged = records.clone();
        assert_eq!(
            forge_transfer(&mut forged[live].payload, &open, forge),
            1,
            "{name}: the live root holds the open transfer once"
        );
        let error = restore(&forged).expect_err(name);
        assert!(error.message.contains("link history"), "{name}: {error:?}");
        let Err(error) = start(forged) else {
            panic!("{name}: a run started from a forged root");
        };
        assert!(error.message.contains("link history"), "{name}: {error:?}");
    }
}
