//! Production roots by scope, the batch command, residues, and the packets
//! that drive the later production systems.
//!
//! A world splits its sites into scope roots, each a whole production root
//! under its own record: a batch of one holder's operations against one root
//! settles item by item with one outcome each, and leaves every other root
//! unread and unwritten. Each root enforces its own configured limits as
//! recorded rejections. What no execution or facility project archives (a
//! work order that ended without an execution, an outcome naming nothing
//! hot) leaves the root as a residue with the next archive commit.
//!
//! Only a packet that changes a root drives its incident evaluation and its
//! reports: refused packets draw nothing and publish nothing, and the root
//! keeps the incident receipts of its latest incident boundary only. A
//! commit that another plugin schedules and production's checks refuse is a
//! recorded refusal, not a failed boundary.

#![allow(clippy::trivially_copy_pass_by_ref, clippy::too_many_lines)]

mod support;

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, Command, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, IngressClass, IngressPayload, Issuer, PayloadSchema, PersonId,
    PluginIngressDescriptor, PluginIngressRequest, PluginIngressTarget, PluginRegistrar,
    SimDuration, SimEvent, SimTime, SimulationPlugin, SimulationView, StateKey, SystemCadence,
};
use canwu_production::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use support::*;

fn scope(name: &str) -> ProductionScopeId {
    ProductionScopeId::new(format!("production:{name}")).expect("scope")
}

fn root(canwu: &Canwu, scope: Option<&ProductionScopeId>) -> ProductionState {
    production_root_state(canwu, scope)
        .expect("production root query")
        .expect("production root")
}

fn root_version(canwu: &Canwu, scope: Option<&ProductionScopeId>) -> u64 {
    let reference = scope.map_or_else(
        production_runtime_reference,
        production_scope_runtime_reference,
    );
    canwu
        .typed_domain_record(&reference)
        .expect("production root record")
        .version
}

/// A run over every root of `states`, with `archive` as its archive
/// provider.
fn world(plugin: &ProductionPlugin, states: Vec<ProductionState>, archive: &TestArchive) -> Canwu {
    let mut scenario = scenario(ProductionState::default());
    scenario.domain_records = states
        .into_iter()
        .map(|state| state.into_initial_record().expect("production root"))
        .collect();
    let mut canwu = Canwu::new_with_plugins(61, scenario, &[plugin]).expect("world");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    canwu
}

fn item(id: &str, operation: ProductionOperation) -> ProductionBatchItemV1 {
    ProductionBatchItemV1 {
        operation_id: ProductionOperationOutcomeId::new(format!("production:batch:{id}"))
            .expect("operation ID"),
        operation,
    }
}

fn order_id(name: &str) -> WorkOrderId {
    WorkOrderId::new(format!("production:order:{name}")).expect("order")
}

fn create(name: &str) -> ProductionBatchItemV1 {
    item(
        &format!("create:{name}"),
        ProductionOperation::CreateWorkOrder {
            work_order: WorkOrder {
                id: order_id(name),
                holder: holder(),
                process: process().id,
                site: site_id(),
                quantity: 1,
                lifecycle: WorkOrderLifecycle::Proposed,
                execution: None,
                expected_revision: 1,
                created_at: SimTime::EPOCH,
            },
        },
    )
}

fn authorize(name: &str) -> ProductionBatchItemV1 {
    item(
        &format!("authorize:{name}"),
        ProductionOperation::AuthorizeWorkOrder {
            work_order: order_id(name),
        },
    )
}

fn cancel(name: &str) -> ProductionBatchItemV1 {
    item(
        &format!("cancel:{name}"),
        ProductionOperation::CancelWorkOrder {
            work_order: order_id(name),
        },
    )
}

/// Queues one tracked batch command. Its boundary admits it, and the next
/// boundary settles the batch.
fn enqueue_batch(
    canwu: &mut Canwu,
    request: u64,
    scope: Option<&ProductionScopeId>,
    expected_runtime_revision: Option<u64>,
    items: Vec<ProductionBatchItemV1>,
) {
    let batch = ProductionBatchEnvelopeV1 {
        holder: holder(),
        scope: scope.cloned(),
        expected_runtime_revision,
        items,
    };
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(1)),
                    Command::Plugin {
                        plugin: PLUGIN_NAME.to_owned(),
                        command: PRODUCTION_BATCH_COMMAND.to_owned(),
                        payload: serde_json::to_value(batch).expect("batch payload"),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("tracked batch command");
}

fn last_command_rejection(canwu: &Canwu) -> Option<String> {
    match &canwu
        .command_attempts()
        .last()
        .expect("command attempt")
        .outcome
    {
        CommandAttemptOutcome::Rejected { error } => Some(error.message.clone()),
        CommandAttemptOutcome::Accepted { .. } => None,
    }
}

fn dispositions(
    state: &ProductionState,
    items: &[ProductionBatchItemV1],
) -> Vec<Option<ProductionOperationDisposition>> {
    items
        .iter()
        .map(|item| {
            state
                .operation_outcomes
                .get(&item.operation_id)
                .map(|outcome| outcome.disposition)
        })
        .collect()
}

fn events_of(events: &[SimEvent], event_type: &str) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind.plugin_identity() == Some((PLUGIN_NAME, event_type)))
        .map(|event| event.summary.clone())
        .collect()
}

#[test]
fn scope_roots_settle_batches_alone_and_archive_their_residues() {
    use ProductionOperationDisposition::{Applied, Rejected};
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let (county_a, county_b) = (scope("county-a"), scope("county-b"));
    // Every root holds the same site and facility identities: roots share
    // nothing, so identities are per root.
    let mut root_a = base_state();
    root_a.scope = Some(county_a.clone());
    root_a.limits.max_work_orders = 3;
    root_a.limits.max_batch_items = 6;
    let mut root_b = base_state();
    root_b.scope = Some(county_b.clone());
    let mut canwu = world(&plugin, vec![base_state(), root_a, root_b], &archive);
    let primary_version = root_version(&canwu, None);
    let b_version = root_version(&canwu, Some(&county_b));

    // One batch: the fourth creation exceeds the root's configured work-order
    // bound and is a recorded rejection; every other item applies in order.
    let items = vec![
        create("w1"),
        authorize("w1"),
        create("w2"),
        create("w3"),
        create("w4"),
        cancel("w2"),
    ];
    let events = canwu.events().len();
    enqueue_batch(&mut canwu, 1, Some(&county_a), None, items.clone());
    settle(&mut canwu);
    settle(&mut canwu);
    let settled = root(&canwu, Some(&county_a));
    assert_eq!(
        dispositions(&settled, &items),
        [Applied, Applied, Applied, Applied, Rejected, Applied].map(Some)
    );
    assert!(!settled.work_orders.contains_key(&order_id("w4")));
    assert_eq!(
        settled.work_orders[&order_id("w2")].lifecycle,
        WorkOrderLifecycle::Cancelled
    );
    let summaries = events_of(
        &canwu.events()[events..],
        "canwu.production.batch_settled.v1",
    );
    assert_eq!(summaries.len(), 1, "{summaries:?}");
    assert!(
        summaries[0].contains("5 applied, 1 rejected"),
        "{summaries:?}"
    );
    // The other roots were not written, and the activity record names the
    // one root the batch changed.
    assert_eq!(root_version(&canwu, None), primary_version);
    assert_eq!(root_version(&canwu, Some(&county_b)), b_version);
    let activity = canwu
        .typed_domain_record(&production_boundary_activity_reference())
        .expect("activity record")
        .decode_payload::<ProductionBoundaryActivityRecord>()
        .expect("activity payload");
    assert_eq!(activity.roots.len(), 1);
    assert_eq!(activity.roots[0].scope, Some(county_a.clone()));
    assert!(activity.roots[0].trigger.is_some());

    // The same batch again finds every item settled and changes nothing.
    enqueue_batch(&mut canwu, 2, Some(&county_a), None, items.clone());
    settle(&mut canwu);
    settle(&mut canwu);
    assert_eq!(root(&canwu, Some(&county_a)), settled);

    // A batch expecting another revision than the root's, behind or ahead,
    // is refused whole and records nothing, so it can be sent again.
    let stale = vec![create("w5")];
    for (request, expected) in [(3, settled.revision - 1), (4, settled.revision + 2)] {
        let events = canwu.events().len();
        enqueue_batch(
            &mut canwu,
            request,
            Some(&county_a),
            Some(expected),
            stale.clone(),
        );
        settle(&mut canwu);
        settle(&mut canwu);
        let refused = events_of(
            &canwu.events()[events..],
            "canwu.production.ingress_refused.v1",
        );
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(
            refused[0].contains("expected runtime revision"),
            "{refused:?}"
        );
        assert_eq!(root(&canwu, Some(&county_a)), settled);
    }
    // Sent again without an expected revision, it settles: past the root's
    // work-order bound, as a recorded rejection.
    enqueue_batch(&mut canwu, 5, Some(&county_a), None, stale.clone());
    settle(&mut canwu);
    settle(&mut canwu);
    let after_stale = root(&canwu, Some(&county_a));
    assert_eq!(dispositions(&after_stale, &stale), [Some(Rejected)]);
    assert_eq!(
        after_stale.operation_outcomes[&stale[0].operation_id]
            .rejection_code
            .as_deref(),
        Some("invalid_domain_record")
    );
    // A batch past the root's item bound, and one naming no installed root,
    // are refused at admission.
    let oversized = (0..7).map(|index| create(&format!("x{index}"))).collect();
    enqueue_batch(&mut canwu, 6, Some(&county_a), None, oversized);
    settle(&mut canwu);
    assert!(
        last_command_rejection(&canwu).is_some_and(|reason| reason.contains("batch item bound"))
    );
    enqueue_batch(
        &mut canwu,
        7,
        Some(&scope("county-z")),
        None,
        vec![create("z")],
    );
    settle(&mut canwu);
    assert!(last_command_rejection(&canwu).is_some_and(|reason| reason.contains("not configured")));
    assert_eq!(root(&canwu, Some(&county_a)), after_stale);

    // Residues: the order cancelled without an execution leaves with its
    // outcomes, and the rejected creations, which name no hot order, leave on
    // their own.
    let prepared = after_stale
        .prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, &archive)
        .expect("residue commit");
    assert_eq!(
        prepared.selected_residues,
        vec![
            ProductionTerminalArchiveKeyV1::WorkOrder(order_id("w2")),
            ProductionTerminalArchiveKeyV1::OperationOutcome(items[4].operation_id.clone()),
            ProductionTerminalArchiveKeyV1::OperationOutcome(stale[0].operation_id.clone()),
        ]
    );
    let receipt =
        enqueue_production_archive(&mut canwu, &prepared, &archive).expect("enqueue residues");
    assert_eq!(receipt.scope, Some(county_a.clone()));
    settle(&mut canwu);
    let archived = root(&canwu, Some(&county_a));
    assert_eq!(archived.archive.archived_residue_count, 3);
    assert!(!archived.work_orders.contains_key(&order_id("w2")));
    for gone in [&items[2], &items[4], &items[5], &stale[0]] {
        assert!(!archived.operation_outcomes.contains_key(&gone.operation_id));
    }
    assert!(archived.residue_archive_candidates().is_empty());
    let Some(ProductionArchivedRecordV1::Residue(record)) = archived_production_terminal_record(
        &archived,
        &archive,
        &ProductionTerminalArchiveKeyV1::WorkOrder(order_id("w2")),
    )
    .expect("residue lookup") else {
        panic!("the cancelled order resolves as a residue");
    };
    assert_eq!(
        record
            .operation_outcomes
            .iter()
            .map(|outcome| outcome.id.clone())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([items[2].operation_id.clone(), items[5].operation_id.clone()])
    );
    assert!(root(&canwu, None).archive.directory_root.is_none());

    // A scope root's completion leases and output keys carry its scope
    // prefix, so no two roots name one resource-side key; a lease without it
    // is a recorded rejection.
    let mut request = lease_request(
        "unscoped",
        canwu_resource::ResourceOperationKey::new("resource:output-key:unscoped").expect("key"),
        SimTime::EPOCH,
    );
    let unscoped = item(
        "lease:unscoped",
        ProductionOperation::RequestCompletionLease {
            request: request.clone(),
        },
    );
    request.id = canwu_resource::CompletionLeaseAcquisitionId::new(format!(
        "{county_a}/production:completion-acquisition:scoped"
    ))
    .expect("acquisition");
    enqueue_batch(&mut canwu, 8, Some(&county_a), None, vec![unscoped.clone()]);
    settle(&mut canwu);
    settle(&mut canwu);
    let outcome = &root(&canwu, Some(&county_a)).operation_outcomes[&unscoped.operation_id];
    assert_eq!(outcome.disposition, Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_payload"));
    assert!(
        outcome
            .rejection_message
            .as_deref()
            .is_some_and(|message| message.contains("scope prefix")),
        "{outcome:?}"
    );
    assert!(production_root_names_key(
        Some(&county_a),
        request.id.as_str()
    ));
    assert!(!production_root_names_key(None, request.id.as_str()));

    // Restore and exact replay agree with every root.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored = from_production_snapshot_json_with_archives(
        &snapshot,
        &[&plugin],
        Arc::new(archive.clone()),
        Arc::new(archive.clone()),
    )
    .expect("restore");
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    let replayed = replay_production_from_journal_with_archives(
        &[&plugin],
        &canwu.replay_journal(),
        Arc::new(archive.clone()),
        Arc::new(archive.clone()),
    )
    .expect("exact replay");
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}

/// The base root with a facility whose incident is certain and an operator
/// who sees its site at once: any boundary that drives the root draws and
/// publishes.
fn certain_incident_state() -> ProductionState {
    let mut state = base_state();
    {
        let facility = state.facilities.get_mut(&facility_id()).expect("facility");
        facility.incident_risk_per_mille = 1_000;
        facility.incident_max_severity_per_mille = 1;
    }
    let grant =
        ProductionObserverGrantId::new("production:observer-grant:operator").expect("grant");
    state.observer_grants.insert(
        grant.clone(),
        ProductionObserverGrant {
            id: grant,
            holder: holder(),
            sites: BTreeSet::from([site_id()]),
            role: ProductionObservationRole::Operator,
            delay_minutes: 0,
        },
    );
    state
}

#[test]
fn only_settled_packets_drive_incidents_and_reports() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let state = certain_incident_state();
    let mut canwu = runtime(&plugin, state, &archive);
    let installed = production_state(&canwu);

    // Junk the host can queue: an acknowledgement that names the root but
    // does not decode, and an operation packet no tracked command admitted.
    for (packet_type, payload) in [
        (
            PRODUCTION_OUTPUT_ACK_INGRESS,
            serde_json::json!({
                "production_source": { "record": production_runtime_reference().into_untyped() },
            }),
        ),
        (
            "production_operation_v1",
            serde_json::json!({ "envelope": 1 }),
        ),
    ] {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                packet_type,
                SimTime::EPOCH,
                payload,
            ))
            .expect("the host queues a public packet");
    }
    settle(&mut canwu);
    settle(&mut canwu);
    assert!(
        canwu.random_draws().is_empty(),
        "refused packets draw nothing"
    );
    assert!(
        canwu
            .knowledge()
            .records
            .get(&holder())
            .is_none_or(canwu_api::HolderLedger::is_empty)
    );
    assert_eq!(production_state(&canwu), installed);

    // Each applied operation evaluates the facility's certain incident and
    // publishes the site's report; the root keeps the receipts of its latest
    // incident boundary and folds the earlier ones into its history.
    for round in 0..3_u64 {
        enqueue_operation(
            &mut canwu,
            10 + round,
            &format!("production:create:incident-{round}"),
            create(&format!("incident-{round}")).operation,
        );
        settle(&mut canwu);
        settle(&mut canwu);
    }
    let after = production_state(&canwu);
    assert_eq!(canwu.random_draws().len(), 6);
    assert_eq!(after.incident_receipts.len(), 1);
    assert_eq!(after.incident_history.folded_receipts, 2);
    assert!(!canwu.knowledge().records[&holder()].is_empty());
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored = from_production_snapshot_json(&snapshot, &[&plugin]).expect("restore");
    assert_eq!(production_state(&restored), after);
    let replayed =
        replay_production_from_journal(&[&plugin], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}

/// Queues a production packet that does not decode, so the boundary runs
/// production's event-driven systems while settling nothing.
fn enqueue_junk_packet(canwu: &mut Canwu) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            PRODUCTION_OUTPUT_ACK_INGRESS,
            SimTime::EPOCH,
            serde_json::json!({
                "production_source": { "record": production_runtime_reference().into_untyped() },
            }),
        ))
        .expect("the host queues a public packet");
}

/// A boundary activity record installed with the scenario, naming the
/// boundary that runs next and the primary root with a trigger, drives
/// nothing: after the domain commit only a version the boundary itself wrote
/// is trusted.
#[test]
fn a_planted_boundary_activity_record_drives_nothing() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let state = certain_incident_state();
    let mut probe = runtime(&plugin, state.clone(), &archive);
    enqueue_junk_packet(&mut probe);
    settle(&mut probe);
    let first = probe.boundaries().last().expect("the first boundary").id;

    let planted = ProductionBoundaryActivityV1 {
        boundary: first,
        roots: vec![ProductionRootActivityV1 {
            scope: None,
            trigger: Some(canwu_api::IngressId::new(1)),
        }],
    };
    let mut scenario = scenario(state);
    scenario.domain_records.push(canwu_api::DomainRecord {
        reference: production_boundary_activity_reference().into_untyped(),
        owner: PLUGIN_NAME.to_owned(),
        class: canwu_api::DomainRecordClass::Record,
        version: 1,
        lifecycle: canwu_api::DomainRecordLifecycle::Active,
        payload: serde_json::to_value(&planted).expect("planted activity"),
        references: Vec::new(),
    });
    let mut canwu = Canwu::new_with_plugins(57, scenario, &[&plugin]).expect("world");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    let installed = production_state(&canwu);
    // The boundary the record names runs production's systems on a packet
    // that settles nothing.
    enqueue_junk_packet(&mut canwu);
    settle(&mut canwu);
    assert_eq!(
        canwu.boundaries().last().map(|boundary| boundary.id),
        Some(first)
    );
    settle(&mut canwu);
    assert!(
        canwu.random_draws().is_empty(),
        "a planted record draws nothing"
    );
    assert!(
        canwu
            .knowledge()
            .records
            .get(&holder())
            .is_none_or(canwu_api::HolderLedger::is_empty),
        "a planted record publishes nothing"
    );
    assert_eq!(production_state(&canwu), installed);
}

/// A plugin that, on its own packet, schedules production packets whose
/// content production's checks refuse: an archive commit that selects
/// nothing, a completion step naming another boundary, a continuation
/// witness for no project, and a retention acknowledgement for no handle.
struct ForgingPlugin;

const FORGE_PACKET: &str = "forge_v1";

const FORGED_TARGETS: [&str; 4] = [
    PRODUCTION_ARCHIVE_COMMIT_INGRESS,
    PRODUCTION_COMPLETION_INGRESS,
    PRODUCTION_RESOURCE_CONTINUATION_INGRESS,
    PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS,
];

fn forged_payloads() -> [serde_json::Value; 4] {
    let digest = "a".repeat(64);
    let commit = VerifiedProductionArchiveCommitV1 {
        scope: None,
        expected_source_root: digest.clone(),
        selected_receipts: Vec::new(),
        selected: Vec::new(),
        selected_projects: Vec::new(),
        selected_residues: Vec::new(),
        directory_root: digest.clone(),
        lookup_root: digest.clone(),
        retention: ProductionArchiveRetentionHandleV1 {
            handle_id: digest.clone(),
            source_root: digest.clone(),
            target_directory_root: digest.clone(),
            object_ids: BTreeMap::new(),
            phase: ProductionArchiveRetentionPhaseV1::Verified,
            semantic_digest: String::new(),
        },
    };
    let step = ProductionCompletionIngressV1::Activate {
        acquisition: canwu_resource::CompletionLeaseAcquisitionId::new(
            "production:completion-acquisition:forged",
        )
        .expect("acquisition"),
        expected_acquisition_revision: canwu_resource::ResourceRevision::INITIAL,
        current_boundary: 999,
    };
    let witness = ProductionResourceContinuationWitnessV1 {
        project: FacilityProjectId::new("production:project:forged").expect("project"),
        resource_archive_directory_root: digest.clone(),
        resource_archive_record_count: 1,
        input_bindings_digest: digest.clone(),
        semantic_digest: digest.clone(),
    };
    [
        serde_json::json!({ "commit": commit }),
        serde_json::to_value(step).expect("step"),
        serde_json::to_value(witness).expect("witness"),
        serde_json::json!({ "handle_id": digest }),
    ]
}

fn forge_packets(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin { plugin, .. } = &ingress.payload
            && plugin == "fixture-forger"
        {
            for (packet_type, payload) in FORGED_TARGETS.into_iter().zip(forged_payloads()) {
                directives.push(BoundaryDirective::SchedulePluginIngress {
                    target_plugin: PLUGIN_NAME.to_owned(),
                    after: SimDuration::ZERO,
                    packet_type: packet_type.to_owned(),
                    priority: 0,
                    payload,
                    affected: Vec::new(),
                });
            }
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for ForgingPlugin {
    fn name(&self) -> &'static str {
        "fixture-forger"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: FORGE_PACKET.to_owned(),
            description: "Schedule forged production packets".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut system = BoundarySystemContract::new(
            "fixture_forge_packets_v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        system.reads = vec![StateKey::core_ingress()];
        system.plugin_ingress_targets = FORGED_TARGETS
            .into_iter()
            .map(|packet_type| PluginIngressTarget {
                target_plugin: PLUGIN_NAME.to_owned(),
                packet_type: packet_type.to_owned(),
            })
            .collect();
        registrar.register_boundary_system(system, forge_packets)
    }
}

#[test]
fn forged_packets_another_plugin_schedules_are_recorded_refusals() {
    let plugin = ProductionPlugin;
    let forger = ForgingPlugin;
    let archive = TestArchive::default();
    let mut canwu =
        Canwu::new_with_plugins(67, scenario(base_state()), &[&plugin, &forger]).expect("runtime");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    let installed = production_state(&canwu);
    let version = root_version(&canwu, None);
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "fixture-forger",
            FORGE_PACKET,
            SimTime::EPOCH,
            serde_json::json!({}),
        ))
        .expect("forge packet");
    let events = canwu.events().len();
    settle(&mut canwu);
    settle(&mut canwu);
    // The commit, the step, and the witness are refused with a trace; the
    // acknowledgement names no handle and changes nothing.
    let refused = events_of(
        &canwu.events()[events..],
        "canwu.production.ingress_refused.v1",
    );
    assert_eq!(refused.len(), 3, "{refused:?}");
    for packet_type in &FORGED_TARGETS[..3] {
        assert!(
            refused.iter().any(|summary| summary.contains(packet_type)),
            "{packet_type}: {refused:?}"
        );
    }
    assert_eq!(production_state(&canwu), installed);
    assert_eq!(root_version(&canwu, None), version);
    // The run goes on.
    enqueue_operation(
        &mut canwu,
        1,
        "production:create:after-forgery",
        create("after-forgery").operation,
    );
    settle(&mut canwu);
    settle(&mut canwu);
    assert!(
        production_state(&canwu)
            .work_orders
            .contains_key(&order_id("after-forgery"))
    );
}

/// Queues one tracked command whose payload is `payload` as given, expecting
/// the simulation revision `expected`.
fn enqueue_raw(
    canwu: &mut Canwu,
    request: u64,
    expected: u64,
    command: &str,
    payload: serde_json::Value,
) {
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request),
                expected,
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(1)),
                    Command::Plugin {
                        plugin: PLUGIN_NAME.to_owned(),
                        command: command.to_owned(),
                        payload,
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("tracked command");
}

fn single(state: &ProductionState, id: &str, operation: ProductionOperation) -> serde_json::Value {
    serde_json::to_value(ProductionCommandEnvelope {
        operation_id: ProductionOperationOutcomeId::new(id).expect("operation ID"),
        holder: holder(),
        expected_runtime_revision: state.revision,
        operation,
        scope: None,
    })
    .expect("envelope")
}

/// A facility project whose progress exceeds its total: admission's outcome
/// accounting refuses it.
fn malformed_project() -> ProductionOperation {
    let digest = "b".repeat(64);
    ProductionOperation::CreateFacilityProject {
        project: FacilityProject {
            id: FacilityProjectId::new("production:project:malformed").expect("project"),
            holder: holder(),
            site: site_id(),
            facility: facility_id(),
            kind: FacilityProjectKind::Repair,
            process: process().id,
            lifecycle: FacilityProjectLifecycle::Planned,
            completed_units: 5,
            total_units: 1,
            base_generation: 1,
            resulting_generation: 2,
            evidence: Vec::new(),
            technology: TechnologyEvidenceBinding {
                technique_revision: version::<canwu_technology::TechniqueRevision>(
                    "technology:forged-technique:v1",
                ),
                capability_qualification: None,
                implementation: None,
                adoption: None,
                semantic_digest: digest.clone(),
            },
            inputs: Vec::new(),
            operation_key: canwu_resource::ResourceOperationKey::new("production:project-key")
                .expect("key"),
            completion_certificate: canwu_resource::CompletionLeaseActivationCertificateV1 {
                acquisition: canwu_resource::CompletionLeaseAcquisitionId::new(
                    "production:completion-acquisition:malformed",
                )
                .expect("acquisition"),
                acquisition_revision: canwu_resource::ResourceRevision::INITIAL,
                operation_key: canwu_resource::ResourceOperationKey::new("production:project-key")
                    .expect("key"),
                prepared_grants: Vec::new(),
                locked_target_versions: Vec::new(),
                recipe_digest: digest.clone(),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope_digest: digest.clone(),
                activation_boundary: 1,
                semantic_digest: digest,
            },
            production_completion_grant: canwu_resource::CompletionCapacityGrantId::new(
                "production:grant:malformed",
            )
            .expect("grant"),
            resource_completion_grant: canwu_resource::CompletionCapacityGrantId::new(
                "resource:grant:malformed",
            )
            .expect("grant"),
            resulting_asset: None,
            created_at: SimTime::EPOCH,
            started_at: None,
            completed_at: None,
            result_evidence_digest: None,
        },
    }
}

#[test]
fn malformed_and_non_canonical_commands_are_command_rejections() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut canwu = runtime(&plugin, base_state(), &archive);
    let installed = production_state(&canwu);

    // A malformed item of a single command or a batch is the command's
    // recorded rejection; the boundary settles.
    let simulation = canwu.revision();
    enqueue_raw(
        &mut canwu,
        1,
        simulation,
        PRODUCTION_COMMAND,
        single(&installed, "production:project:single", malformed_project()),
    );
    settle(&mut canwu);
    assert!(
        last_command_rejection(&canwu).is_some(),
        "the single command is rejected"
    );
    let simulation = canwu.revision();
    enqueue_raw(
        &mut canwu,
        2,
        simulation,
        PRODUCTION_BATCH_COMMAND,
        serde_json::to_value(ProductionBatchEnvelopeV1 {
            holder: holder(),
            scope: None,
            expected_runtime_revision: None,
            items: vec![create("ok"), item("project", malformed_project())],
        })
        .expect("batch"),
    );
    settle(&mut canwu);
    assert!(
        last_command_rejection(&canwu).is_some(),
        "the batch is rejected"
    );

    // A payload that is not its canonical encoding (here an explicit null)
    // is rejected at admission instead of being dropped at settlement.
    let mut payload = single(
        &installed,
        "production:create:null",
        create("null").operation,
    );
    payload
        .as_object_mut()
        .expect("envelope object")
        .insert("scope".to_owned(), serde_json::Value::Null);
    let simulation = canwu.revision();
    enqueue_raw(&mut canwu, 3, simulation, PRODUCTION_COMMAND, payload);
    settle(&mut canwu);
    assert!(
        last_command_rejection(&canwu).is_some_and(|reason| reason.contains("canonical encoding"))
    );

    // A degraded-facility choice that no persisted decision executes is a
    // recorded command rejection, not a failed boundary.
    let simulation = canwu.revision();
    enqueue_raw(
        &mut canwu,
        4,
        simulation,
        PRODUCTION_COMMAND,
        single(
            &installed,
            "production:degraded:undecided",
            ProductionOperation::ResolveDegradedFacility {
                work_order: order_id("undecided"),
                facility: facility_id(),
                choice: DegradedFacilityChoice::ContinueDegraded,
                decision_ticket: canwu_api::DecisionTicketId::new(1),
            },
        ),
    );
    settle(&mut canwu);
    assert!(
        last_command_rejection(&canwu)
            .is_some_and(|reason| reason.contains("persisted Canwu decision")),
        "{:?}",
        last_command_rejection(&canwu)
    );
    settle(&mut canwu);
    assert_eq!(production_state(&canwu), installed);
}

#[test]
fn an_operation_without_outcome_room_at_settlement_is_refused() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = base_state();
    state.limits.max_operation_outcomes = 1;
    let mut canwu = runtime(&plugin, state, &archive);
    let installed = production_state(&canwu);
    // Both commands find the one free slot at admission; the second finds it
    // taken when it settles and records nothing.
    let simulation = canwu.revision();
    for (offset, name) in [(0_u64, "first"), (1, "second")] {
        enqueue_raw(
            &mut canwu,
            10 + offset,
            simulation + offset,
            PRODUCTION_COMMAND,
            single(
                &ProductionState {
                    revision: installed.revision + offset,
                    ..ProductionState::default()
                },
                &format!("production:create:{name}"),
                create(name).operation,
            ),
        );
    }
    let events = canwu.events().len();
    settle(&mut canwu);
    settle(&mut canwu);
    let after = production_state(&canwu);
    assert_eq!(after.operation_outcomes.len(), 1);
    assert!(after.work_orders.contains_key(&order_id("first")));
    assert!(!after.work_orders.contains_key(&order_id("second")));
    let refused = events_of(
        &canwu.events()[events..],
        "canwu.production.ingress_refused.v1",
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
}

#[test]
fn a_batch_past_the_operations_a_root_settles_in_one_boundary_is_refused_whole() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = base_state();
    state.limits.max_operations_per_boundary = 2;
    // A batch item bound above the operation bound would admit batches that
    // can never settle, so a root with it does not validate.
    let mut unsettleable = state.clone();
    unsettleable.limits.max_batch_items = 4;
    assert!(unsettleable.limits.validate().is_err());
    assert!(unsettleable.into_initial_record().is_err());
    state.limits.max_batch_items = 2;
    let mut canwu = runtime(&plugin, state, &archive);
    // One command and a batch of two settle in one boundary: the command
    // takes one of the root's two operations, and the batch, which would
    // take it to three, is refused whole and records nothing.
    let simulation = canwu.revision();
    enqueue_operation(
        &mut canwu,
        10,
        "production:create:first",
        create("first").operation,
    );
    let batch = vec![create("third"), create("fourth")];
    enqueue_raw(
        &mut canwu,
        11,
        simulation + 1,
        PRODUCTION_BATCH_COMMAND,
        serde_json::to_value(ProductionBatchEnvelopeV1 {
            holder: holder(),
            scope: None,
            expected_runtime_revision: None,
            items: batch.clone(),
        })
        .expect("batch payload"),
    );
    let events = canwu.events().len();
    settle(&mut canwu);
    settle(&mut canwu);
    let after = production_state(&canwu);
    assert!(after.work_orders.contains_key(&order_id("first")));
    assert!(!after.work_orders.contains_key(&order_id("third")));
    assert_eq!(after.operation_outcomes.len(), 1);
    let refused = events_of(
        &canwu.events()[events..],
        "canwu.production.ingress_refused.v1",
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("past the operations it settles in one boundary"),
        "{refused:?}"
    );
    // Sent again, it settles in a boundary of its own.
    enqueue_batch(&mut canwu, 12, None, None, batch);
    settle(&mut canwu);
    settle(&mut canwu);
    let resent = production_state(&canwu);
    assert!(resent.work_orders.contains_key(&order_id("third")));
    assert!(resent.work_orders.contains_key(&order_id("fourth")));
    assert_eq!(resent.operation_outcomes.len(), 3);

    // Three single commands in one boundary: the third finds the bound
    // reached and is refused with a trace.
    let simulation = canwu.revision();
    for (offset, name) in [(0_u64, "fifth"), (1, "sixth"), (2, "seventh")] {
        enqueue_raw(
            &mut canwu,
            20 + offset,
            simulation + offset,
            PRODUCTION_COMMAND,
            single(
                &ProductionState {
                    revision: resent.revision + offset,
                    ..ProductionState::default()
                },
                &format!("production:create:{name}"),
                create(name).operation,
            ),
        );
    }
    let events = canwu.events().len();
    settle(&mut canwu);
    settle(&mut canwu);
    let singles = production_state(&canwu);
    assert!(singles.work_orders.contains_key(&order_id("sixth")));
    assert!(!singles.work_orders.contains_key(&order_id("seventh")));
    assert_eq!(singles.operation_outcomes.len(), 5);
    let refused = events_of(
        &canwu.events()[events..],
        "canwu.production.ingress_refused.v1",
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("operation bound for this boundary"),
        "{refused:?}"
    );
}

/// A plugin that publishes, on its own packet and before production's report
/// phase, every record the boundary's publication budget has left.
struct FloodPlugin;

const FLOOD_PACKET: &str = "flood_v1";

fn flood_schema() -> canwu_api::KnowledgeSchemaId {
    canwu_api::KnowledgeSchemaId::new(
        canwu_api::KnowledgeRecordKind::new("fixture.flood", "note"),
        1,
    )
}

// A boundary system has this signature whether or not it can fail.
#[allow(clippy::unnecessary_wraps)]
fn flood(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    let flooded = context.admitted_ingress.iter().any(|ingress_id| {
        view.ingress(*ingress_id).ok().flatten().is_some_and(|ingress| {
            matches!(&ingress.payload, IngressPayload::Plugin { plugin, .. } if plugin == "fixture-flood")
        })
    });
    if flooded {
        let budget = view.knowledge_publication_budget();
        let mut left = budget.records.min(budget.holder_entries);
        let mut index = 0_u64;
        while left > 0 {
            let chunk = left.min(budget.records_per_batch);
            let records = (0..chunk)
                .map(|_| {
                    index += 1;
                    canwu_api::KnowledgeRecordDraft {
                        schema: flood_schema(),
                        subjects: Vec::new(),
                        payload: serde_json::json!(index),
                        as_of: None,
                        confidence_per_mille: 1_000,
                        origin: canwu_api::KnowledgeOrigin {
                            method: "fixture_flood".to_owned(),
                            evidence: Vec::new(),
                        },
                        supersedes: Vec::new(),
                        contradicts: Vec::new(),
                    }
                })
                .collect();
            directives.push(BoundaryDirective::PublishKnowledge {
                visibility: canwu_api::StateVisibility::SameBoundary,
                producer_correlation: None,
                supersession: canwu_api::KnowledgeSupersession::Explicit,
                groups: vec![canwu_api::KnowledgePublicationGroup {
                    holders: vec![holder()],
                    records,
                }],
                summary: "Spend the publication budget".to_owned(),
            });
            left -= chunk;
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for FloodPlugin {
    fn name(&self) -> &'static str {
        "fixture-flood"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f2"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let schema = canwu_api::PluginKnowledgeSchema {
            id: flood_schema(),
            schema_hash: "f".repeat(64),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
            retention: canwu_api::KnowledgeRetention::Keep,
        };
        registrar.register_knowledge_schema(schema.clone())?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: FLOOD_PACKET.to_owned(),
            description: "Spend the boundary's publication budget".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut system = BoundarySystemContract::new(
            "fixture_flood_v1",
            BoundaryPhase::PerceptionAndAttentionRefresh,
            SystemCadence::EventDriven,
        );
        system.reads = vec![StateKey::core_ingress(), StateKey::core_knowledge()];
        system.knowledge_writes = vec![canwu_api::KnowledgeWriteGrant {
            schema: schema.id,
            visibilities: vec![canwu_api::StateVisibility::SameBoundary],
        }];
        system.visibility = canwu_api::StateVisibility::SameBoundary;
        registrar.register_boundary_system(system, flood)
    }
}

#[test]
fn a_report_past_the_publication_budget_waits_and_publishes_on_its_wake() {
    let plugin = ProductionPlugin;
    let flooder = FloodPlugin;
    let mut state = base_state();
    let grant =
        ProductionObserverGrantId::new("production:observer-grant:operator").expect("grant");
    state.observer_grants.insert(
        grant.clone(),
        ProductionObserverGrant {
            id: grant,
            holder: holder(),
            sites: BTreeSet::from([site_id()]),
            role: ProductionObservationRole::Operator,
            delay_minutes: 0,
        },
    );
    let mut canwu =
        Canwu::new_with_plugins(71, scenario(state), &[&plugin, &flooder]).expect("runtime");
    let reports = |canwu: &Canwu| {
        canwu
            .knowledge()
            .records
            .get(&holder())
            .map_or(0, |records| {
                records
                    .values()
                    .filter(|record| record.schema == production_report_knowledge_schema_id())
                    .count()
            })
    };
    // An applied operation makes the site's report due in the boundary the
    // flood spends the whole publication budget in.
    enqueue_operation(
        &mut canwu,
        1,
        "production:create:report",
        create("report").operation,
    );
    settle(&mut canwu);
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "fixture-flood",
            FLOOD_PACKET,
            SimTime::EPOCH,
            serde_json::json!({}),
        ))
        .expect("flood packet");
    let events = canwu.events().len();
    settle(&mut canwu);
    assert!(
        production_state(&canwu)
            .work_orders
            .contains_key(&order_id("report"))
    );
    assert_eq!(reports(&canwu), 0, "the report waits past the budget");
    let deferred = events_of(
        &canwu.events()[events..],
        "canwu.production.report_capacity_rejected.v1",
    );
    assert!(
        deferred
            .iter()
            .any(|summary| summary.contains("publication budget")),
        "{deferred:?}"
    );
    // Its wake, one minute later, publishes it.
    canwu
        .settle_boundary(canwu_api::BoundaryRequest::at(SimTime::from_minutes(1)))
        .expect("wake boundary");
    assert_eq!(reports(&canwu), 1);
}

fn with_operator_grant(mut state: ProductionState, delay_minutes: u64) -> ProductionState {
    let grant =
        ProductionObserverGrantId::new("production:observer-grant:operator").expect("grant");
    state.observer_grants.insert(
        grant.clone(),
        ProductionObserverGrant {
            id: grant,
            holder: holder(),
            sites: BTreeSet::from([site_id()]),
            role: ProductionObservationRole::Operator,
            delay_minutes,
        },
    );
    state
}

/// One `CreateWorkOrder` created at the run's current time.
fn create_now(canwu: &Canwu, name: &str) -> ProductionOperation {
    let mut operation = create(name).operation;
    if let ProductionOperation::CreateWorkOrder { work_order } = &mut operation {
        work_order.created_at = canwu.time();
    }
    operation
}

#[test]
fn reports_whose_heads_left_the_root_still_restore() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut canwu = runtime(&plugin, with_operator_grant(base_state(), 0), &archive);
    // Twenty reports of one holder and site: the root keeps the sixteen
    // newest heads, and the reports of the older ones are kept for life.
    for round in 0..20_u64 {
        let operation = create_now(&canwu, &format!("head-{round}"));
        enqueue_operation(
            &mut canwu,
            100 + round,
            &format!("production:create:head-{round}"),
            operation,
        );
        settle(&mut canwu);
        settle(&mut canwu);
    }
    let state = production_state(&canwu);
    assert_eq!(
        state
            .observation_heads
            .values()
            .map(Vec::len)
            .sum::<usize>(),
        MAX_OBSERVATION_HEADS_PER_SCOPE
    );
    assert_eq!(canwu.knowledge().records[&holder()].len(), 20);
    validate_production_runtime(&canwu).expect("every report restores");
    let snapshot = canwu.snapshot_json().expect("snapshot");
    from_production_snapshot_json(&snapshot, &[&plugin]).expect("restore");
}

/// A pair blocked only by another pair's rolled-out head takes the room that
/// head's due prune frees in the same boundary.
#[test]
fn a_pair_blocked_by_another_pairs_rolled_out_head_settles_when_it_is_pruned() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = with_operator_grant(base_state(), 20);
    let local = canwu_api::KnowledgeHolderRef::Entity(canwu_api::EntityRef::Government(
        canwu_api::GovernmentId::new(1),
    ));
    let grant = ProductionObserverGrantId::new("production:observer-grant:local").expect("grant");
    state.observer_grants.insert(
        grant.clone(),
        ProductionObserverGrant {
            id: grant,
            holder: local.clone(),
            sites: BTreeSet::from([site_id()]),
            role: ProductionObservationRole::LocalOwner,
            delay_minutes: 20,
        },
    );
    state.limits.max_observation_rollover = 1;
    let mut canwu = runtime(&plugin, state, &archive);
    let minute = |value: i64| SimTime::from_minutes(value);
    let local_key = ProductionObservationHeadKeyV1 {
        holder: local,
        scope: site_id(),
    };
    // One head a minute for eighteen minutes for both pairs: the operator's
    // seventeenth head rolls its first out and fills the shared rollover, so
    // the local owner's newest head waits behind it.
    for round in 0..18_i64 {
        canwu
            .settle_boundary(canwu_api::BoundaryRequest::at(minute(round)))
            .expect("clock");
        let operation = create_now(&canwu, &format!("shared-{round}"));
        enqueue_operation(
            &mut canwu,
            300 + u64::try_from(round).expect("round"),
            &format!("production:create:shared-{round}"),
            operation,
        );
        settle(&mut canwu);
        settle(&mut canwu);
    }
    let waiting = production_state(&canwu);
    assert_eq!(waiting.observation_rollover.len(), 1);
    assert!(waiting.observation_dirty_index.contains(&local_key));
    // Each pair keeps one due time, its newest head's. The operator's comes at
    // minute 41, when its rolled-out head is pruned; that prune runs before
    // the waiting pair is visited, so the pair settles in that boundary
    // instead of waiting for the root's next activity, as no later wake
    // would come.
    while canwu.step_canonical().expect("due wake boundary").is_some() {
        assert!(canwu.time() <= minute(120), "the wakes end");
    }
    let settled = production_state(&canwu);
    assert!(
        !settled.observation_dirty_index.contains(&local_key),
        "the waiting pair settles when the other pair's head is pruned"
    );
    assert!(settled.observation_rollover.is_empty());
    assert!(
        settled
            .observation_heads
            .values()
            .flatten()
            .any(|head| head.key == local_key && head.observed_at == minute(41))
    );
}

#[test]
fn a_pair_waiting_on_a_full_rollover_drains_when_its_heads_come_due() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = with_operator_grant(base_state(), 20);
    state.limits.max_observation_rollover = 1;
    let mut canwu = runtime(&plugin, state, &archive);
    let minute = |value: i64| SimTime::from_minutes(value);
    // One head a minute for eighteen minutes: the seventeenth rolls the
    // first out and fills the rollover, and the eighteenth waits, because no
    // head of the pair is old enough to be read yet.
    for round in 0..18_i64 {
        canwu
            .settle_boundary(canwu_api::BoundaryRequest::at(minute(round)))
            .expect("clock");
        let operation = create_now(&canwu, &format!("rolling-{round}"));
        enqueue_operation(
            &mut canwu,
            200 + u64::try_from(round).expect("round"),
            &format!("production:create:rolling-{round}"),
            operation,
        );
        settle(&mut canwu);
        settle(&mut canwu);
    }
    let waiting = production_state(&canwu);
    assert_eq!(waiting.observation_rollover.len(), 1);
    assert_eq!(
        waiting.observation_dirty_index.len(),
        1,
        "the newest head waits"
    );
    // With no more activity, the pair's wakes reach minute 21, when its second
    // head becomes readable: the first is pruned and the waiting head is
    // materialized.
    while canwu.time() < minute(22) && canwu.step_canonical().expect("due wake boundary").is_some()
    {
    }
    let drained = production_state(&canwu);
    assert!(drained.observation_dirty_index.is_empty());
    assert!(
        drained
            .observation_heads
            .values()
            .flatten()
            .any(|head| head.observed_at >= minute(18))
    );
}
