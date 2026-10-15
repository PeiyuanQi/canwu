//! Gap G-50: the run keeps its index of operation-keyed draws with its
//! state instead of rebuilding it at every boundary and copying it for every
//! due system. This fixture pins what the kept index must answer, with
//! several systems drawing in each boundary: an address drawn before names
//! its recorded draw and is not drawn again, a reused address with another
//! bound, purpose, or evidence is refused, and an address a failed boundary
//! drew is forgotten. A fork, a restored run, a replayed run, a sealed run,
//! and a run restored from sealed evidence all answer alike, although the
//! sealed runs no longer retain the evidence the draws cite. That the index
//! is kept and not rebuilt is pinned in `canwu-sim` (the kept index shares
//! its tree across a boundary and a fork), and the cost is measured by
//! `benchmarks/performance-harness` (`keyed-draw-profile`).

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CompactedCanwu, ErrorCode, EvidenceRef,
    IngressClass, IngressId, IngressPayload, PayloadSchema, PluginIngressDescriptor,
    PluginIngressRequest, PluginRegistrar, RandomOperationTarget, RandomStreamKey, Scenario,
    SimDuration, SimEvent, SimTime, SimulationPlugin, SimulationView, StateKey, SystemCadence,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const PLUGIN: &str = "fixture-keyed-index";
const ZONES: [&str; 3] = ["zone-a", "zone-b", "zone-c"];

/// A random stream has one owning system, so each zone draws from its own.
fn stream(zone: &str) -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, zone, 1)
}

/// Each zone makes one keyed draw for each admitted request and reports the
/// draw it was given, or that the index refused the address. A request cites
/// as operation evidence the packet its payload names, or else the packet
/// that carries it.
fn draw_requests(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut proposal = BoundaryProposal::default();
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin { payload, .. } = &record.payload else {
            continue;
        };
        let text = |field: &str| payload[field].as_str().unwrap_or_default();
        let operation = text("operation");
        let cited = payload["evidence"].as_u64().map_or(*id, IngressId::new);
        let summary = match view.random_draw_for_operation(
            &stream(&context.system),
            EvidenceRef::Ingress(cited),
            "draw",
            operation,
            RandomOperationTarget::CanonicalKey(operation.to_owned()),
            0,
            payload["bound"].as_u64().unwrap_or_default(),
            text("purpose"),
        ) {
            Ok(draw) => format!(
                "{} {operation} draw={} value={}",
                context.system,
                draw.draw_id.get(),
                draw.sample.value
            ),
            Err(error) if error.code == ErrorCode::RandomOperationConflict => {
                format!("{} {operation} refused", context.system)
            }
            Err(error) => return Err(error),
        };
        proposal.directives.push(BoundaryDirective::Emit {
            event_type: "answered".to_owned(),
            summary,
            affected: Vec::new(),
        });
    }
    Ok(proposal)
}

/// Fails every boundary that carries the Monthly cadence, after the draws.
fn veto(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Err(CanwuError::new(
        ErrorCode::InvalidBoundary,
        "the fixture vetoes Monthly boundaries",
    ))
}

struct KeyedIndexPlugin;

impl SimulationPlugin for KeyedIndexPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a50"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "draw".to_owned(),
            description: "every zone draws once for the named operation".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        for zone in ZONES {
            let mut contract = BoundarySystemContract::new(
                zone,
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::EventDriven,
            );
            contract.reads = vec![StateKey::core_ingress()];
            contract.random_streams = vec![stream(zone)];
            contract.emits = vec!["answered".to_owned()];
            registrar.register_boundary_system(contract, draw_requests)?;
        }
        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "veto",
                BoundaryPhase::PerspectiveAndReportMaterialization,
                SystemCadence::Monthly,
            ),
            veto,
        )
    }
}

/// The request every address of the fixture is first drawn with. It cites
/// the packet that carries it.
fn first_request(operation: &str) -> Value {
    json!({ "operation": operation, "bound": 1_000, "purpose": "first" })
}

/// The same request again, citing the packet the address was first drawn for.
fn repeat_request(operation: &str, packets: &BTreeMap<String, IngressId>) -> Value {
    let mut request = first_request(operation);
    request["evidence"] = json!(packets[operation].get());
    request
}

fn answers(events: &[SimEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind.field("event_type") == Some(json!("answered")))
        .map(|event| event.summary.clone())
        .collect()
}

/// Settles one boundary an hour on that asks for two addresses drawn before,
/// three reuses of drawn addresses with another bound, purpose, or evidence,
/// and one new address. Returns what the zones answered and how many draws
/// the boundary added to the journal.
macro_rules! probe {
    ($runtime:expr, $packets:expr) => {{
        let runtime = &mut $runtime;
        let packets: &BTreeMap<String, IngressId> = $packets;
        let at = runtime.time() + SimDuration::hours(1);
        let cursor = runtime.evidence_cursor().expect("evidence cursor");
        let mut changed = ["op-04", "op-05", "op-06"].map(|op| repeat_request(op, packets));
        changed[0]["bound"] = json!(999);
        changed[1]["purpose"] = json!("second");
        changed[2]["evidence"] = json!(packets["op-00"].get());
        for payload in [
            repeat_request("op-03", packets),
            repeat_request("ghost-1", packets),
        ]
        .into_iter()
        .chain(changed)
        .chain([first_request("probe")])
        {
            runtime
                .enqueue_plugin_ingress(PluginIngressRequest::new(PLUGIN, "draw", at, payload))
                .expect("the probe request should enqueue");
        }
        runtime
            .settle_boundary(BoundaryRequest::at(at))
            .expect("the probe boundary should settle");
        let tail = runtime
            .journal_segment_since(cursor)
            .expect("the probe boundary's evidence");
        (answers(&tail.events), tail.random_draws.len())
    }};
}

#[test]
fn gap_g50_sim_keyed_draw_index() {
    let plugin = KeyedIndexPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let mut canwu =
        Canwu::new_with_plugins(50, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the keyed index plugin should register");
    // The packet each operation was first requested with.
    let mut packets = BTreeMap::new();
    let mut enqueue = |canwu: &mut Canwu, at: SimTime, operations: &[String]| {
        for operation in operations {
            let receipt = canwu
                .enqueue_plugin_ingress(PluginIngressRequest::new(
                    PLUGIN,
                    "draw",
                    at,
                    first_request(operation),
                ))
                .expect("the request should enqueue");
            packets.insert(operation.clone(), receipt.ingress_id);
        }
    };

    // Thirty operations in three zones: ninety addresses in one boundary.
    let operations: Vec<_> = (0..30).map(|index| format!("op-{index:02}")).collect();
    enqueue(&mut canwu, SimTime::EPOCH, &operations);
    let first = canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the first boundary should settle");
    assert_eq!(first.random_draws.len(), 90);

    // A boundary that fails after its systems drew is forgotten: the retry
    // draws the same addresses as new ones and journals them.
    let ghosts = ["ghost-0".to_owned(), "ghost-1".to_owned()];
    let later = SimTime::EPOCH + SimDuration::hours(1);
    enqueue(&mut canwu, later, &ghosts);
    let error = canwu
        .settle_boundary(BoundaryRequest::at(later).with_cadence(SystemCadence::Monthly))
        .expect_err("the Monthly veto should fail the boundary");
    assert_eq!(error.code, ErrorCode::InvalidBoundary);
    assert_eq!(canwu.random_draws().len(), 90);
    let retried = canwu
        .settle_boundary(BoundaryRequest::at(later))
        .expect("the retry should settle");
    assert_eq!(retried.random_draws.len(), 6);
    let drawn = answers(canwu.events());
    assert_eq!(drawn.len(), 96);
    // A direct boundary admits the emitted events, so that the evidence can
    // be sealed below.
    canwu
        .settle_boundary(BoundaryRequest::at(later))
        .expect("a direct boundary should settle");

    // The live answer, on a fork: drawn addresses name their first draw and
    // add nothing to the journal, each changed reuse is refused in every
    // zone, and only the new address is drawn.
    let mut fork = canwu.fork();
    let (answered, new_draws) = probe!(fork, &packets);
    let first_answer = |zone: &str, operation: &str| {
        drawn
            .iter()
            .find(|line| line.starts_with(&format!("{zone} {operation} draw=")))
            .cloned()
            .expect("the address was drawn before")
    };
    assert_eq!(answered.len(), 18);
    for (zone, lines) in ZONES.into_iter().zip(answered.chunks(6)) {
        assert_eq!(
            lines[..5],
            [
                first_answer(zone, "op-03"),
                first_answer(zone, "ghost-1"),
                format!("{zone} op-04 refused"),
                format!("{zone} op-05 refused"),
                format!("{zone} op-06 refused"),
            ]
        );
        assert!(lines[5].starts_with(&format!("{zone} probe draw=")));
    }
    assert_eq!(new_draws, 3);
    let live = (answered, new_draws);

    // The fork's draws stayed in the fork, and a restored and a replayed run
    // rebuild an index that answers alike.
    let mut restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().expect("snapshot"), &plugins)
            .expect("the run should load");
    assert_eq!(probe!(restored, &packets), live);
    let journal = serde_json::to_string(&canwu.replay_journal()).expect("journal json");
    let mut replayed =
        Canwu::replay_from_journal_json(&plugins, &journal).expect("the run should replay");
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(probe!(replayed, &packets), live);
    let mut reloaded = Canwu::from_checkpoint_journal_with_plugins(
        canwu.checkpoint_journal().expect("checkpoint journal"),
        &plugins,
    )
    .expect("the checkpoint journal should load");
    assert_eq!(probe!(reloaded, &packets), live);

    // After a seal every draw record and every packet the draws cite has
    // left the runtime; the keyed-draw ledger keeps every address and the
    // checkpoint lists none. The sealed run and a run restored from the
    // sealed evidence still answer as the unsealed runs do.
    let mut compacted = canwu.into_compacted().expect("compact runtime");
    let sealed = compacted
        .seal_evidence()
        .expect("the retained evidence should seal")
        .expect("the retained evidence is not empty");
    let checkpoint = compacted.checkpoint().expect("sealed checkpoint");
    let wire = serde_json::to_value(&checkpoint).expect("checkpoint encodes");
    assert!(wire.get("keyed_draw_reservations").is_none());
    let mut resumed = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        checkpoint,
        vec![sealed],
        &plugins,
    )
    .expect("the sealed run should restore");
    assert_eq!(probe!(compacted, &packets), live);
    assert_eq!(probe!(resumed, &packets), live);
}
