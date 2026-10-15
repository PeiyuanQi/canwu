//! A first operation-keyed draw, and the existence and time of the evidence
//! it cites, must not depend on what the run has sealed.
//!
//! A fresh draw may cite a committed event, command, attempt, ingress record,
//! boundary, or draw whether or not a seal moved it out of the runtime, and a
//! record version only while it is its record's current (or initial) version.
//! An unsealed run, the same run after a seal, and a run restored from the
//! sealed segments therefore draw the same values under the same IDs and
//! refuse the same draws, and report the same existence and time for sealed
//! evidence.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CompactedCanwu, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    DomainRecordVersionRef, DomainRecordVersionSource, EventId, EvidenceRef, IngressClass,
    IngressId, IngressPayload, PayloadSchema, PluginIngressDescriptor, PluginIngressRequest,
    PluginRegistrar, RandomOperationTarget, RandomStreamKey, Scenario, SimDuration, SimEvent,
    SimTime, SimulationPlugin, SimulationView, StateKey, SystemCadence,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-sealed-evidence";

fn ledger_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "ledger")
}

fn ledger() -> DomainRecordRef {
    DomainRecordRef {
        kind: ledger_kind(),
        id: "ledger".to_owned(),
    }
}

fn ledger_state() -> StateKey {
    DomainRecordSchema::new(ledger_kind(), DomainRecordClass::Record).state_key()
}

fn stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "draws", 1)
}

fn admitted<'a>(
    view: &'a SimulationView<'_>,
    context: &BoundaryContext,
    packet_type: &str,
) -> Result<Vec<(IngressId, &'a Value)>, CanwuError> {
    let mut packets = Vec::new();
    for id in &context.admitted_ingress {
        if let Some(record) = view.ingress(*id)?
            && let IngressPayload::Plugin {
                packet_type: actual,
                payload,
                ..
            } = &record.payload
            && actual == packet_type
        {
            packets.push((*id, payload));
        }
    }
    Ok(packets)
}

/// Creates the ledger record or moves it to its next version.
fn bump(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if admitted(view, context, "bump")?.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let count = view
        .domain_record(&ledger())?
        .map_or(0, |record| record.version);
    let record = DomainRecordDraft::new(ledger(), json!({ "count": count + 1 }));
    let mutation = if count == 0 {
        DomainRecordMutation::Create { record }
    } else {
        DomainRecordMutation::Update {
            record,
            expected_version: count,
        }
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation,
            summary: "The ledger moves on".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// The evidence a draw request names: its own packet, or the reference in
/// its payload.
fn cited(
    view: &SimulationView<'_>,
    id: IngressId,
    payload: &Value,
) -> Result<EvidenceRef, CanwuError> {
    let cite = &payload["cite"];
    let number = |field: &str| cite[field].as_u64().unwrap_or_default();
    Ok(match cite["kind"].as_str().unwrap_or("self") {
        "ingress" => EvidenceRef::Ingress(IngressId::new(number("id"))),
        "event" => EvidenceRef::Event(EventId::new(number("id"))),
        "boundary" => EvidenceRef::Boundary(canwu_api::BoundaryId::new(number("id"))),
        "version" => EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
            record: ledger(),
            version: number("version"),
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: canwu_api::BoundaryId::new(number("boundary")),
                change_index: number("change"),
            },
        }),
        "current" => EvidenceRef::DomainRecordVersion(
            view.current_domain_record_version(&ledger())?
                .expect("the ledger exists"),
        ),
        _ => EvidenceRef::Ingress(id),
    })
}

/// Makes one keyed draw for each admitted request and emits what it got:
/// the draw, or the code of the refusal.
fn draw(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut proposal = BoundaryProposal::default();
    for (id, payload) in admitted(view, context, "draw")? {
        let operation = payload["operation"].as_str().unwrap_or_default();
        let evidence = cited(view, id, payload)?;
        let summary = match view.random_draw_for_operation(
            &stream(),
            evidence,
            "draw",
            operation,
            RandomOperationTarget::CanonicalKey(operation.to_owned()),
            0,
            1_000_000,
            "sealed-evidence fixture draw",
        ) {
            Ok(drawn) => format!(
                "{operation} draw={} value={}",
                drawn.draw_id.get(),
                drawn.sample.value
            ),
            Err(error) => format!("{operation} refused={:?}", error.code),
        };
        proposal.directives.push(BoundaryDirective::Emit {
            event_type: "drew".to_owned(),
            summary,
            affected: Vec::new(),
        });
    }
    Ok(proposal)
}

struct SealedEvidencePlugin;

impl SimulationPlugin for SealedEvidencePlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000016a1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            ledger_kind(),
            DomainRecordClass::Record,
        ))?;
        for name in ["bump", "draw"] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: format!("fixture {name} request"),
                class: IngressClass::Information,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        let mut bumping = BoundarySystemContract::new(
            "a-bump",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        bumping.reads = vec![StateKey::core_ingress(), ledger_state()];
        bumping.writes = vec![ledger_state()];
        registrar.register_boundary_system(bumping, bump)?;
        let mut drawing = BoundarySystemContract::new(
            "b-draw",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        drawing.reads = vec![
            StateKey::core_ingress(),
            StateKey::core_events(),
            StateKey::core_evidence(),
            ledger_state(),
        ];
        drawing.random_streams = vec![stream()];
        drawing.emits = vec!["drew".to_owned()];
        registrar.register_boundary_system(drawing, draw)
    }
}

fn hour(hour: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::hours(hour)
}

fn request(at: SimTime, packet_type: &str, payload: Value) -> PluginIngressRequest {
    PluginIngressRequest::new(PLUGIN, packet_type, at, payload)
}

fn drew(events: &[SimEvent], after: SimTime) -> Vec<String> {
    events
        .iter()
        .filter(|event| {
            event.timestamp >= after && event.kind.field("event_type") == Some(json!("drew"))
        })
        .map(|event| event.summary.clone())
        .collect()
}

/// The same host input for every run: first draws citing sealed evidence,
/// draws citing the current and a superseded version, and a retry.
fn later_requests(early_packet: IngressId, first_event: EventId) -> Vec<PluginIngressRequest> {
    let at = hour(4);
    vec![
        request(
            at,
            "draw",
            json!({"operation": "cites-ingress", "cite": {"kind": "ingress", "id": early_packet.get()}}),
        ),
        request(
            at,
            "draw",
            json!({"operation": "cites-event", "cite": {"kind": "event", "id": first_event.get()}}),
        ),
        request(
            at,
            "draw",
            json!({"operation": "cites-boundary", "cite": {"kind": "boundary", "id": 1}}),
        ),
        request(
            at,
            "draw",
            json!({"operation": "cites-current", "cite": {"kind": "current"}}),
        ),
        request(
            at,
            "draw",
            json!({"operation": "cites-superseded", "cite": {"kind": "version", "version": 2, "boundary": 2, "change": 0}}),
        ),
        request(
            at,
            "draw",
            json!({"operation": "early", "cite": {"kind": "ingress", "id": early_packet.get()}}),
        ),
    ]
}

#[test]
fn a_first_keyed_draw_and_evidence_identity_do_not_depend_on_sealing() {
    let plugin = SealedEvidencePlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let mut unsealed =
        Canwu::new_with_plugins(61, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the fixture should start");

    // Ledger version 1 at boundary 1, version 2 at boundary 2 beside an
    // early draw, version 3 at boundary 3, and an idle boundary that admits
    // the last emissions so the history can be sealed.
    unsealed
        .enqueue_plugin_ingress(request(hour(0), "bump", json!({})))
        .expect("bump");
    unsealed
        .settle_boundary(BoundaryRequest::at(hour(0)))
        .expect("boundary 1");
    unsealed
        .enqueue_plugin_ingress(request(hour(1), "bump", json!({})))
        .expect("bump");
    let early_packet = unsealed
        .enqueue_plugin_ingress(request(hour(1), "draw", json!({"operation": "early"})))
        .expect("early draw")
        .ingress_id;
    let second = unsealed
        .settle_boundary(BoundaryRequest::at(hour(1)))
        .expect("boundary 2");
    let early = drew(unsealed.events(), hour(1));
    assert_eq!(early.len(), 1, "the early request drew");
    unsealed
        .enqueue_plugin_ingress(request(hour(2), "bump", json!({})))
        .expect("bump");
    unsealed
        .settle_boundary(BoundaryRequest::at(hour(2)))
        .expect("boundary 3");
    unsealed
        .settle_boundary(BoundaryRequest::at(hour(3)))
        .expect("idle boundary");
    let first_event = unsealed.events()[0].id;
    let superseded = DomainRecordVersionRef {
        record: ledger(),
        version: 2,
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: second.boundary_id,
            change_index: 0,
        },
    };
    assert_eq!(second.boundary_id.get(), 2);
    assert!(unsealed.domain_record_version_evidence_exists(&superseded));

    // Seal everything; restore a third run from the sealed segment.
    let mut sealed = unsealed.fork().into_compacted().expect("compact runtime");
    let segment = sealed
        .seal_evidence()
        .expect("the history should seal")
        .expect("the history is not empty");
    assert!(sealed.events().is_empty() && sealed.ingress_log().is_empty());
    for reference in [
        EvidenceRef::Ingress(early_packet),
        EvidenceRef::Event(first_event),
        EvidenceRef::Boundary(canwu_api::BoundaryId::new(1)),
    ] {
        assert!(
            sealed.archived_evidence_receipt(&reference).is_none(),
            "{reference:?} is sealed without a receipt"
        );
    }
    let mut restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment],
        &plugins,
    )
    .expect("the sealed run should restore");

    // Existence and time of sealed ordinal evidence agree across the runs.
    for reference in [
        EvidenceRef::Ingress(early_packet),
        EvidenceRef::Event(first_event),
        EvidenceRef::Boundary(canwu_api::BoundaryId::new(1)),
    ] {
        let expected = (
            unsealed.evidence_exists(&reference),
            unsealed.evidence_time(&reference),
        );
        assert!(expected.0 && expected.1.is_some());
        assert_eq!(
            (
                sealed.evidence_exists(&reference),
                sealed.evidence_time(&reference)
            ),
            expected,
            "sealed run on {reference:?}"
        );
        assert_eq!(
            (
                restored.evidence_exists(&reference),
                restored.evidence_time(&reference)
            ),
            expected,
            "restored run on {reference:?}"
        );
    }

    // The same later requests draw and refuse alike.
    for request in later_requests(early_packet, first_event) {
        unsealed
            .enqueue_plugin_ingress(request.clone())
            .expect("draw");
        sealed
            .enqueue_plugin_ingress(request.clone())
            .expect("draw");
        restored.enqueue_plugin_ingress(request).expect("draw");
    }
    let unsealed_receipt = unsealed
        .settle_boundary(BoundaryRequest::at(hour(4)))
        .expect("unsealed");
    let sealed_receipt = sealed
        .settle_boundary(BoundaryRequest::at(hour(4)))
        .expect("sealed");
    let restored_receipt = restored
        .settle_boundary(BoundaryRequest::at(hour(4)))
        .expect("restored");
    let outcomes = drew(unsealed.events(), hour(4));
    assert_eq!(drew(sealed.events(), hour(4)), outcomes);
    assert_eq!(drew(restored.events(), hour(4)), outcomes);
    assert_eq!(sealed_receipt, unsealed_receipt);
    assert_eq!(restored_receipt, unsealed_receipt);
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(restored.checkpoint_hash(), unsealed.checkpoint_hash());
    let state_hash = unsealed.authoritative_state_hash().expect("state hash");
    assert_eq!(
        sealed.authoritative_state_hash().expect("state hash"),
        state_hash
    );
    assert_eq!(
        restored.authoritative_state_hash().expect("state hash"),
        state_hash
    );

    let by_operation = |operation: &str| {
        outcomes
            .iter()
            .find(|outcome| outcome.starts_with(&format!("{operation} ")))
            .cloned()
            .expect("every request reports an outcome")
    };
    for operation in [
        "cites-ingress",
        "cites-event",
        "cites-boundary",
        "cites-current",
    ] {
        assert!(
            by_operation(operation).contains(" draw="),
            "{operation} draws in every run"
        );
    }
    assert_eq!(
        by_operation("cites-superseded"),
        "cites-superseded refused=InvalidRandomOperationEvidence",
        "a superseded version is refused in every run, sealed or not"
    );
    assert_eq!(
        by_operation("early"),
        early[0],
        "a retry returns the early draw"
    );
}
