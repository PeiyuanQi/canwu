//! Gap G-01: a queued plugin ingress item can be withdrawn by its issuer
//! before its due time, never settles, and survives save/load and replay.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, ErrorCode, IngressCancellationAuthority,
    IngressClass, IngressId, IngressPayload, IngressReceipt, PayloadSchema,
    PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest, PluginRegistrar, Scenario,
    SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey, SystemCadence,
};
use serde_json::{Value, json};
use std::sync::OnceLock;

const TIMER_PLUGIN: &str = "fixture-timer";
static TICK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static TOCK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

fn descriptor(name: &str) -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: name.to_owned(),
        description: format!("fixture {name} packet"),
        class: IngressClass::Communication,
        payload_schema: PayloadSchema::Any,
    }
}

fn keep_permit(
    slot: &OnceLock<PluginIngressPermit>,
    permit: PluginIngressPermit,
) -> Result<(), CanwuError> {
    match slot.set(permit) {
        Err(permit) if slot.get() != Some(&permit) => Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "fixture permit changed across registrations",
        )),
        _ => Ok(()),
    }
}

/// Delivers packets, arms a boundary-scheduled timeout and internal tick, and
/// withdraws the timeout from inside a later boundary when it is disarmed.
fn settle_timers(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut proposal = BoundaryProposal::default();
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin { packet_type, .. } = &record.payload else {
            continue;
        };
        match packet_type.as_str() {
            "arm" => {
                for (packet_type, days) in [("timeout", 5), ("tick", 6)] {
                    proposal
                        .directives
                        .push(BoundaryDirective::ScheduleIngress {
                            after: SimDuration::days(days),
                            packet_type: packet_type.to_owned(),
                            priority: 0,
                            payload: json!({ "timer": "watch" }),
                            affected: Vec::new(),
                        });
                }
            }
            "disarm" => {
                for pending in view.cancellable_plugin_ingress()? {
                    if matches!(
                        &pending.payload,
                        IngressPayload::Plugin { packet_type, .. } if packet_type == "timeout"
                    ) {
                        proposal
                            .directives
                            .push(BoundaryDirective::CancelPluginIngress {
                                ingress_id: pending.id,
                                reason: "timer disarmed".to_owned(),
                            });
                    }
                }
            }
            _ => proposal.directives.push(BoundaryDirective::Emit {
                event_type: "delivered".to_owned(),
                summary: format!("delivered {packet_type} ingress {id}"),
                affected: Vec::new(),
            }),
        }
    }
    Ok(proposal)
}

struct TimerPlugin;

impl SimulationPlugin for TimerPlugin {
    fn name(&self) -> &'static str {
        TIMER_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a01"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for name in ["ping", "arm", "disarm", "timeout"] {
            registrar.register_ingress(descriptor(name))?;
        }
        keep_permit(
            &TICK_PERMIT,
            registrar.register_internal_ingress(descriptor("tick"))?,
        )?;
        keep_permit(
            &TOCK_PERMIT,
            registrar.register_internal_ingress(descriptor("tock"))?,
        )?;
        let mut contract = BoundarySystemContract::new(
            "timers",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        contract.emits = vec!["delivered".to_owned()];
        registrar.register_boundary_system(contract, settle_timers)
    }
}

fn at_day(day: i64) -> SimTime {
    SimTime::EPOCH
        .checked_add(SimDuration::days(day))
        .expect("fixture time should stay in range")
}

fn enqueue(canwu: &mut Canwu, packet_type: &str, day: i64) -> IngressReceipt {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            TIMER_PLUGIN,
            packet_type,
            at_day(day),
            json!({ "packet": packet_type, "day": day }),
        ))
        .expect("host plugin ingress should enqueue")
}

fn error_code(result: Result<IngressReceipt, CanwuError>) -> ErrorCode {
    result
        .expect_err("the cancellation should be rejected")
        .code
}

fn admitted(canwu: &Canwu, id: IngressId) -> bool {
    canwu
        .boundaries()
        .iter()
        .any(|boundary| boundary.admitted_ingress.contains(&id))
}

fn cancellation_of(
    canwu: &Canwu,
    receipt: &IngressReceipt,
) -> (IngressId, IngressCancellationAuthority) {
    let record = canwu
        .ingress_log()
        .iter()
        .find(|record| record.id == receipt.ingress_id)
        .expect("the cancellation should be journaled");
    let IngressPayload::PluginCancellation {
        cancelled,
        authority,
        ..
    } = &record.payload
    else {
        panic!("the receipt should name a terminal cancellation record");
    };
    assert_eq!(record.issued_at, receipt.issued_at);
    assert_eq!(record.due_at, record.issued_at);
    (*cancelled, *authority)
}

#[test]
fn gap_g01_sim_cancel_plugin_ingress() {
    let timer = TimerPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&timer];
    let mut canwu =
        Canwu::new_with_plugins(41, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the timer plugin should register");
    let tick_permit = TICK_PERMIT.get().expect("tick permit").clone();
    let foreign_permit = TOCK_PERMIT.get().expect("tock permit").clone();

    enqueue(&mut canwu, "arm", 0);
    let withdrawn_ping = enqueue(&mut canwu, "ping", 2);
    let delivered_ping = enqueue(&mut canwu, "ping", 3);
    let tick = canwu
        .enqueue_permitted_plugin_ingress(
            PluginIngressRequest::new(TIMER_PLUGIN, "tick", at_day(4), json!({ "tick": 1 })),
            &tick_permit,
        )
        .expect("the internal tick should enqueue through its permit");

    // Only the issuer may withdraw: internal packets need their exact permit.
    assert_eq!(
        error_code(canwu.cancel_plugin_ingress(tick.ingress_id, "host override")),
        ErrorCode::InvalidAuthority
    );
    assert_eq!(
        error_code(canwu.cancel_permitted_plugin_ingress(
            tick.ingress_id,
            &foreign_permit,
            "foreign permit"
        )),
        ErrorCode::InvalidAuthority
    );
    let revision_before = canwu.revision();
    let host_cancel = canwu
        .cancel_plugin_ingress(withdrawn_ping.ingress_id, "host withdrew the ping")
        .expect("the host should withdraw its pending ping");
    assert_eq!(
        cancellation_of(&canwu, &host_cancel),
        (
            withdrawn_ping.ingress_id,
            IngressCancellationAuthority::Host
        )
    );
    assert_eq!(canwu.revision(), revision_before);
    assert_eq!(
        error_code(canwu.cancel_plugin_ingress(withdrawn_ping.ingress_id, "again")),
        ErrorCode::LateIngress
    );
    let permit_cancel = canwu
        .cancel_permitted_plugin_ingress(tick.ingress_id, &tick_permit, "plugin withdrew tick")
        .expect("the permit should withdraw its internal tick");
    assert_eq!(
        cancellation_of(&canwu, &permit_cancel),
        (tick.ingress_id, IngressCancellationAuthority::PluginPermit)
    );

    // A boundary system schedules a timeout and an internal tick. The host
    // cannot withdraw the plugin's timeout, but the plugin's permit can
    // withdraw the tick its own boundary system scheduled.
    let armed = canwu
        .advance_canonical(SimDuration::days(1))
        .expect("the arming boundary should settle");
    assert_eq!(armed.len(), 1);
    let [timeout_id, scheduled_tick] = armed[0].generated_ingress[..] else {
        panic!("the arming boundary should schedule a timeout and a tick");
    };
    assert_eq!(
        error_code(canwu.cancel_plugin_ingress(timeout_id, "host override")),
        ErrorCode::InvalidAuthority
    );
    let scheduled_tick_cancel = canwu
        .cancel_permitted_plugin_ingress(scheduled_tick, &tick_permit, "tick no longer needed")
        .expect("the permit should withdraw its plugin's scheduled tick");
    assert_eq!(
        cancellation_of(&canwu, &scheduled_tick_cancel),
        (scheduled_tick, IngressCancellationAuthority::PluginPermit)
    );

    // Its own plugin withdraws it from inside a later boundary.
    enqueue(&mut canwu, "disarm", 1);
    let disarmed = canwu
        .advance_canonical(SimDuration::ZERO)
        .expect("the disarming boundary should settle");
    assert_eq!(disarmed.len(), 1);
    let [directive_cancel] = disarmed[0].generated_ingress[..] else {
        panic!("the disarming boundary should record one terminal cancellation");
    };
    let directive_receipt = IngressReceipt {
        ingress_id: directive_cancel,
        issued_at: at_day(1),
        due_at: at_day(1),
    };
    assert_eq!(
        cancellation_of(&canwu, &directive_receipt),
        (timeout_id, IngressCancellationAuthority::BoundarySystem)
    );

    // Save mid-run, then advance both copies past every withdrawn due time.
    let saved = canwu.snapshot_json().expect("the run should serialize");
    let mut restored = Canwu::from_snapshot_json_with_plugins(&saved, &plugins)
        .expect("pending cancellations should restore");
    for run in [&mut canwu, &mut restored] {
        let settled = run
            .advance_canonical(SimDuration::days(10))
            .expect("the remaining ingress should settle");
        assert_eq!(
            settled
                .iter()
                .map(|receipt| receipt.settled_at)
                .collect::<Vec<_>>(),
            vec![at_day(3)],
            "withdrawn items must not open boundaries at their due times"
        );
    }
    assert_eq!(canwu.snapshot(), restored.snapshot());
    for withdrawn in [
        withdrawn_ping.ingress_id,
        tick.ingress_id,
        timeout_id,
        scheduled_tick,
    ] {
        assert!(
            !admitted(&canwu, withdrawn),
            "{withdrawn} must never settle"
        );
    }
    assert!(admitted(&canwu, delivered_ping.ingress_id));
    let delivered = canwu
        .events()
        .iter()
        .filter(|event| event.kind.field("event_type") == Some(json!("delivered")))
        .count();
    assert_eq!(delivered, 1, "only the surviving ping is delivered");

    // Due or admitted items can no longer be withdrawn.
    assert_eq!(
        error_code(canwu.cancel_plugin_ingress(delivered_ping.ingress_id, "too late")),
        ErrorCode::LateIngress
    );
    let due_now = enqueue(&mut canwu, "ping", 11);
    assert_eq!(
        error_code(canwu.cancel_plugin_ingress(due_now.ingress_id, "already due")),
        ErrorCode::LateIngress
    );
    canwu
        .advance_canonical(SimDuration::ZERO)
        .expect("the due ping should settle");
    assert!(admitted(&canwu, due_now.ingress_id));
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a closing boundary should admit the delivery events");

    // Save/load, checkpoint journals, and exact replay reproduce the run.
    let snapshot_json = canwu.snapshot_json().expect("the run should serialize");
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot_json, &plugins)
        .expect("the finished run should restore");
    assert_eq!(canwu.snapshot(), restored.snapshot());
    let bundle = canwu
        .checkpoint_journal_json()
        .expect("the checkpoint journal should serialize");
    let from_bundle = Canwu::from_checkpoint_journal_json_with_plugins(&bundle, &plugins)
        .expect("the checkpoint journal should restore");
    assert_eq!(canwu.checkpoint_hash(), from_bundle.checkpoint_hash());
    let journal = serde_json::to_string(&canwu.replay_journal()).expect("journal json");
    let replayed = Canwu::replay_from_journal_json(&plugins, &journal)
        .expect("the cancellations should replay exactly");
    assert_eq!(canwu.snapshot(), replayed.snapshot());
    assert_eq!(canwu.checkpoint_hash(), replayed.checkpoint_hash());

    // Settled cancellations do not block evidence sealing.
    let mut compacted = canwu.fork().into_compacted().expect("compact runtime");
    let segment = compacted
        .seal_evidence()
        .expect("terminal cancellations should be sealable")
        .expect("the retained tail should seal");
    let resumed = Canwu::from_checkpoint_and_journal(
        compacted.checkpoint().expect("compact checkpoint"),
        vec![segment],
    )
    .expect("the sealed run should restore");
    assert_eq!(canwu.checkpoint_hash(), resumed.checkpoint_hash());

    // Strict loading still rejects unknown fields on the new record kind.
    let mut tampered: Value = serde_json::from_str(&snapshot_json).expect("snapshot value");
    let record = tampered["ingress"]
        .as_array_mut()
        .expect("ingress journal")
        .iter_mut()
        .find(|record| record["payload"]["type"] == "plugin_cancellation")
        .expect("a cancellation record");
    record["payload"]["unexpected"] = json!(true);
    let rejected = Canwu::from_snapshot_json_with_plugins(&tampered.to_string(), &plugins);
    assert!(
        matches!(rejected, Err(ref error) if error.code == ErrorCode::InvalidSnapshot),
        "unknown cancellation fields must be rejected"
    );
}
