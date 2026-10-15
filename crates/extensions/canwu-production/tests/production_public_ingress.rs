//! The host application can queue the public production packet types, an
//! operation packet and an output acknowledgement among them, and any
//! completion coordinator step through `enqueue_production_completion_operation`,
//! and another plugin can schedule any production packet type. Such a packet
//! settles only when it decodes, when production's checks accept it, and, for
//! an operation, when the admission of a tracked production command queued
//! it; any other packet is refused with a trace, changes nothing, and lets its
//! boundary settle.

mod support;

use canwu_api::{CommandId, PluginIngressRequest, SimEvent, SimTime, canonical_hash};
use canwu_production::*;
use support::*;

/// The public packet type that carries an admitted production operation.
const OPERATION_PACKET: &str = "production_operation_v1";

fn host_packet(packet_type: &str, payload: serde_json::Value) -> PluginIngressRequest {
    PluginIngressRequest::new(PLUGIN_NAME, packet_type, SimTime::EPOCH, payload)
}

fn refusals(events: &[SimEvent], event_type: &str) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind.plugin_identity() == Some((PLUGIN_NAME, event_type)))
        .map(|event| event.summary.clone())
        .collect()
}

#[test]
fn malformed_public_packets_are_refused_and_the_run_continues() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = base_state();
    let order = authorized_order(&mut state, "malformed");
    state.observation_dirty_index.clear();
    state.observation_due_index.clear();
    let mut canwu = runtime(&plugin, state, &archive);
    let ready = production_state(&canwu);

    for (packet_type, payload) in [
        (
            PRODUCTION_OUTPUT_ACK_INGRESS,
            serde_json::json!({ "execution": "x" }),
        ),
        (OPERATION_PACKET, serde_json::json!({ "envelope": 1 })),
    ] {
        canwu
            .enqueue_plugin_ingress(host_packet(packet_type, payload))
            .expect("the host queues a public packet");
    }
    let events = canwu.events().len();
    settle(&mut canwu);
    settle(&mut canwu);
    let refused = &canwu.events()[events..];
    let acknowledgement = refusals(refused, "canwu.production.output_ack_refused.v1");
    assert_eq!(acknowledgement.len(), 1, "{acknowledgement:?}");
    assert!(acknowledgement[0].contains(PRODUCTION_OUTPUT_ACK_INGRESS));
    let operation = refusals(refused, "canwu.production.ingress_refused.v1");
    assert_eq!(operation.len(), 1, "{operation:?}");
    assert!(operation[0].contains(OPERATION_PACKET));
    assert_eq!(production_state(&canwu), ready);

    // The run goes on: a tracked command still settles.
    enqueue_operation(
        &mut canwu,
        1,
        "production:cancel:after-malformed",
        ProductionOperation::CancelWorkOrder {
            work_order: order.clone(),
        },
    );
    settle(&mut canwu);
    settle(&mut canwu);
    assert_eq!(
        production_state(&canwu).work_orders[&order].lifecycle,
        WorkOrderLifecycle::Cancelled
    );
}

#[test]
fn an_operation_packet_no_tracked_command_admitted_is_refused() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = base_state();
    let order = authorized_order(&mut state, "forged");
    state.observation_dirty_index.clear();
    state.observation_due_index.clear();
    let mut canwu = runtime(&plugin, state, &archive);
    let ready = production_state(&canwu);

    // A packet shaped as an admitted operation, with a made-up command: it
    // skipped the issuer's authority, the run policy, the outcome capacity,
    // and the degraded-facility decision that admission checks.
    let envelope = ProductionCommandEnvelope {
        operation_id: ProductionOperationOutcomeId::new("production:cancel:forged")
            .expect("operation ID"),
        holder: holder(),
        expected_runtime_revision: ready.revision,
        operation: ProductionOperation::CancelWorkOrder {
            work_order: order.clone(),
        },
    };
    let forged = serde_json::json!({
        "envelope": envelope,
        "command": CommandId::new(999),
        "input_hash": canonical_hash("canwu.production.operation-input.v1", &envelope)
            .expect("input hash"),
        "decision_receipt": null,
    });
    canwu
        .enqueue_plugin_ingress(host_packet(OPERATION_PACKET, forged))
        .expect("the host queues a public packet");
    let events = canwu.events().len();
    settle(&mut canwu);
    settle(&mut canwu);
    let refused = refusals(
        &canwu.events()[events..],
        "canwu.production.ingress_refused.v1",
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].contains("does not match an admitted tracked production command"));
    // Nothing changed: the order is still authorized and no outcome records
    // the forged operation.
    let after = production_state(&canwu);
    assert_eq!(after, ready);
    assert_eq!(
        after.work_orders[&order].lifecycle,
        WorkOrderLifecycle::Authorized
    );
    assert!(after.operation_outcomes.is_empty());

    // The same operation as a tracked command applies.
    enqueue_operation(
        &mut canwu,
        1,
        "production:cancel:tracked",
        ProductionOperation::CancelWorkOrder {
            work_order: order.clone(),
        },
    );
    settle(&mut canwu);
    settle(&mut canwu);
    let cancelled = production_state(&canwu);
    assert_eq!(
        cancelled.work_orders[&order].lifecycle,
        WorkOrderLifecycle::Cancelled
    );
    let outcome = &cancelled.operation_outcomes
        [&ProductionOperationOutcomeId::new("production:cancel:tracked").expect("operation ID")];
    assert_eq!(outcome.disposition, ProductionOperationDisposition::Applied);

    // Restore and exact replay agree with the run, the refused packet
    // included.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored = from_production_snapshot_json(&snapshot, &[&plugin]).expect("restore");
    assert_eq!(production_state(&restored), cancelled);
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    let replayed =
        replay_production_from_journal(&[&plugin], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(production_state(&replayed), cancelled);
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}

#[test]
fn a_completion_step_production_refuses_is_recorded_and_the_run_continues() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = base_state();
    let lease = activate_lease(&mut state, "host-step");
    state.observation_dirty_index.clear();
    state.observation_due_index.clear();
    let mut canwu = runtime(&plugin, state, &archive);
    let ready = production_state(&canwu);

    // The public API queues any step unchecked. This one names a boundary
    // other than the one that settles it, which the coordinator refuses as
    // `InvalidAuthority`.
    let now = canwu.time();
    enqueue_production_completion_operation(
        &mut canwu,
        now,
        &ProductionCompletionIngressV1::Activate {
            acquisition: lease.certificate.acquisition.clone(),
            expected_acquisition_revision: canwu_resource::ResourceRevision::INITIAL,
            current_boundary: 999,
        },
    )
    .expect("the host queues a completion step");
    let events = canwu.events().len();
    settle(&mut canwu);
    settle(&mut canwu);
    let refused = refusals(
        &canwu.events()[events..],
        "canwu.production.ingress_refused.v1",
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].contains(PRODUCTION_COMPLETION_INGRESS));
    assert_eq!(production_state(&canwu), ready);

    // The run goes on, and restore and exact replay agree with it.
    enqueue_operation(
        &mut canwu,
        1,
        "production:cancel:after-refused-step",
        ProductionOperation::CancelWorkOrder {
            work_order: lease.order.clone(),
        },
    );
    settle(&mut canwu);
    settle(&mut canwu);
    // The tracked command settles to an outcome.
    let after = production_state(&canwu);
    assert!(
        after.operation_outcomes.contains_key(
            &ProductionOperationOutcomeId::new("production:cancel:after-refused-step")
                .expect("operation ID")
        )
    );
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
