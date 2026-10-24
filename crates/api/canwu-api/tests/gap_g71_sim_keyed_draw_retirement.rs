//! Keyed-draw retirement (downstream gap G-71).
//!
//! A random stream that declares a keyed-draw retirement keeps each
//! operation-keyed draw in the keyed-draw ledger until its boundary time plus
//! the declared duration. Until then an exact retry is answered from the
//! ledger and a differing reuse is a conflict; from then on the address
//! draws afresh, under a new draw ID and with the same value for the same
//! bound and purpose. A stream without a retirement keeps its draws. An
//! unsealed run, a sealed run, a run restored from a continuation checkpoint,
//! a snapshot restore, and exact replay retire the same draws, and a
//! continuation checkpoint that lists a retired draw is refused.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CompactedCanwu, ContinuationCheckpoint, ErrorCode,
    EvidenceRef, IngressClass, IngressId, IngressPayload, KeyedDrawRetirement, PayloadSchema,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, RandomOperationTarget,
    RandomStreamKey, Scenario, SimDuration, SimEvent, SimTime, SimulationPlugin, SimulationView,
    StateKey, SystemCadence,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-draw-retirement";

/// Draws of this stream retire two days after their boundary.
fn retiring() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "daily", 1)
}

/// Draws of this stream stay for the run.
fn kept() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "kept", 1)
}

/// Makes one keyed draw per admitted request, citing the ingress record the
/// request names (its own by default), and emits the draw or the refusal.
fn draw(
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
        let operation = payload["operation"].as_str().unwrap_or_default();
        let stream = if payload["kept"].as_bool() == Some(true) {
            kept()
        } else {
            retiring()
        };
        let cited = payload["cite"].as_u64().map_or(*id, IngressId::new);
        let summary = match view.random_draw_for_operation(
            &stream,
            EvidenceRef::Ingress(cited),
            "weather",
            operation,
            RandomOperationTarget::CanonicalKey(operation.to_owned()),
            0,
            1_000_000,
            "retirement fixture draw",
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

struct RetirementPlugin;

impl SimulationPlugin for RetirementPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000007100"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "draw".to_owned(),
            description: "fixture keyed draw request".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut drawing = BoundarySystemContract::new(
            "draw",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        drawing.reads = vec![StateKey::core_ingress()];
        drawing.random_streams = vec![retiring(), kept()];
        drawing.keyed_draw_retirement =
            vec![KeyedDrawRetirement::new(retiring(), SimDuration::days(2))];
        drawing.emits = vec!["drew".to_owned()];
        registrar.register_boundary_system(drawing, draw)
    }
}

fn day(day: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(day)
}

fn drew(events: &[SimEvent], at: SimTime) -> Vec<String> {
    events
        .iter()
        .filter(|event| {
            event.timestamp == at && event.kind.field("event_type") == Some(json!("drew"))
        })
        .map(|event| event.summary.clone())
        .collect()
}

fn request(at: SimTime, payload: Value) -> PluginIngressRequest {
    PluginIngressRequest::new(PLUGIN, "draw", at, payload)
}

/// The same host input for every run on day `d`. Day 0's first request is
/// ingress 1, which the retries cite.
fn requests(d: i64) -> Vec<PluginIngressRequest> {
    let at = day(d);
    match d {
        0 => vec![
            request(at, json!({"operation": "a"})),
            request(at, json!({"operation": "k", "kept": true})),
        ],
        // Before the retirement: an exact retry is answered from the ledger,
        // a reuse with other evidence is a conflict.
        1 => vec![
            request(at, json!({"operation": "a", "cite": 1})),
            request(at, json!({"operation": "a"})),
        ],
        // Day 2 retires day 0's draw before the system runs: the same
        // address draws afresh, citing fresh evidence. The kept stream still
        // answers its retry.
        2 => vec![
            request(at, json!({"operation": "a"})),
            request(at, json!({"operation": "k", "kept": true, "cite": 2})),
        ],
        _ => Vec::new(),
    }
}

/// One of the runs that take the same input.
enum Run {
    Live(Canwu),
    Compact(CompactedCanwu),
}

impl Run {
    fn step(&mut self, d: i64) {
        for request in requests(d) {
            match self {
                Self::Live(run) => run.enqueue_plugin_ingress(request).map(|_| ()),
                Self::Compact(run) => run.enqueue_plugin_ingress(request).map(|_| ()),
            }
            .expect("the request queues");
        }
        match self {
            Self::Live(run) => run.settle_boundary(BoundaryRequest::at(day(d))).map(|_| ()),
            Self::Compact(run) => {
                let settled = run.settle_boundary(BoundaryRequest::at(day(d))).map(|_| ());
                run.seal_evidence().expect("the run seals");
                settled
            }
        }
        .expect("the boundary settles");
    }

    fn drew(&self, d: i64) -> Vec<String> {
        match self {
            Self::Live(run) => drew(run.events(), day(d)),
            Self::Compact(run) => drew(run.events(), day(d)),
        }
    }

    fn checkpoint_hash(&self) -> String {
        match self {
            Self::Live(run) => run.checkpoint_hash().to_owned(),
            Self::Compact(run) => run.checkpoint_hash().to_owned(),
        }
    }

    fn continuation(&self) -> (ContinuationCheckpoint, String) {
        match self {
            Self::Live(run) => run.continuation_checkpoint(),
            Self::Compact(run) => run.continuation_checkpoint(),
        }
        .expect("a continuation checkpoint")
    }
}

/// The draw ID in an outcome such as `a draw=3 value=17`.
fn draw_id(outcome: &str) -> u64 {
    outcome
        .split(" draw=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|id| id.parse().ok())
        .expect("the outcome is a draw")
}

fn value(outcome: &str) -> &str {
    outcome
        .split(" value=")
        .nth(1)
        .expect("the outcome is a draw")
}

#[test]
fn a_keyed_draw_retires_alike_in_every_run() {
    let plugin = RetirementPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let scenario = || Scenario::new(SimTime::EPOCH, Vec::new());
    let new_run = || Canwu::new_with_plugins(71, scenario(), &plugins).expect("the fixture starts");
    let mut runs = vec![
        Run::Live(new_run()),
        Run::Compact(new_run().into_compacted().expect("compact runtime")),
    ];
    let mut outcomes = Vec::new();
    let mut day_one_checkpoint = None;
    for d in 0..4 {
        for run in &mut runs {
            run.step(d);
        }
        let expected = runs[0].drew(d);
        for run in &runs {
            assert_eq!(run.drew(d), expected, "day {d}");
            assert_eq!(run.checkpoint_hash(), runs[0].checkpoint_hash(), "day {d}");
            // An unsealed and a sealed run write the same continuation
            // checkpoint, retired draws left out.
            assert_eq!(run.continuation().1, runs[0].continuation().1, "day {d}");
        }
        if d == 1 {
            // Taken while day 0's draw is still live; a run restored from it
            // retires the draw at day 2 as the others do.
            let (checkpoint, hash) = runs[0].continuation();
            assert_eq!(checkpoint.keyed_draws.len(), 2);
            runs.push(Run::Compact(
                CompactedCanwu::from_continuation_checkpoint_verified(
                    checkpoint.clone(),
                    &hash,
                    &plugins,
                )
                .expect("the day-1 continuation restores"),
            ));
            day_one_checkpoint = Some(checkpoint);
        }
        outcomes.push(expected);
    }

    let first = &outcomes[0][0];
    let kept_first = &outcomes[0][1];
    assert_eq!(
        outcomes[1][0], *first,
        "an exact retry returns the live draw"
    );
    assert_eq!(
        outcomes[1][1], "a refused=RandomOperationConflict",
        "a reuse with other evidence is a conflict until the draw retires"
    );
    let redraw = &outcomes[2][0];
    assert!(
        draw_id(redraw) > draw_id(first),
        "the retired address draws afresh: {redraw} after {first}"
    );
    assert_eq!(
        value(redraw),
        value(first),
        "the same bound and purpose give the same value"
    );
    assert_eq!(
        outcomes[2][1], *kept_first,
        "a stream without a retirement keeps its draws"
    );

    // Day 3's continuation checkpoint lists the re-draw and the kept draw,
    // not the retired one; one that lists the retired draw is refused.
    let (checkpoint, _) = runs[0].continuation();
    let listed: Vec<u64> = checkpoint
        .keyed_draws
        .iter()
        .map(|entry| entry.draw_id.get())
        .collect();
    let mut expected_ids = vec![draw_id(kept_first), draw_id(redraw)];
    expected_ids.sort_unstable();
    assert_eq!(listed, expected_ids);
    let retired = day_one_checkpoint
        .expect("the day-1 checkpoint")
        .keyed_draws
        .into_iter()
        .find(|entry| entry.draw_id.get() == draw_id(first))
        .expect("day 1 lists the first draw");
    let mut tampered = checkpoint.clone();
    tampered.keyed_draws.insert(0, retired);
    tampered.continuation_hash = tampered
        .compute_continuation_hash()
        .expect("the material hashes");
    let error = CompactedCanwu::from_continuation_checkpoint_with_plugins(tampered, &plugins)
        .err()
        .expect("a retired draw in a continuation checkpoint is refused");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A snapshot restore rebuilds the ledger from a journal that draws one
    // address twice, and exact replay reaches the same state.
    let Run::Live(mut live) = runs.swap_remove(0) else {
        unreachable!("the first run is the unsealed one")
    };
    let snapshot = live.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("the snapshot loads");
    assert_eq!(restored.checkpoint_hash(), live.checkpoint_hash());
    let replayed =
        Canwu::replay_from_journal(&plugins, &live.replay_journal()).expect("exact replay");
    assert_eq!(replayed.checkpoint_hash(), live.checkpoint_hash());
    assert_eq!(
        replayed.continuation_checkpoint().expect("checkpoint").1,
        live.continuation_checkpoint().expect("checkpoint").1
    );

    // Time that passes without a boundary drops nothing from the kept
    // ledger, yet a continuation checkpoint lists only the draws not retired
    // at its time: past the re-draw's retirement (day 4), the kept draw
    // alone. It restores, and the restored run settles as the live one.
    live.advance(SimDuration::days(2))
        .expect("time passes without a boundary");
    assert_eq!(live.time(), day(5));
    let (checkpoint, hash) = live.continuation_checkpoint().expect("checkpoint");
    let listed: Vec<u64> = checkpoint
        .keyed_draws
        .iter()
        .map(|entry| entry.draw_id.get())
        .collect();
    assert_eq!(listed, vec![draw_id(kept_first)]);
    let resumed =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("the checkpoint past a retirement restores");
    let (mut live, mut resumed) = (Run::Live(live), Run::Compact(resumed));
    live.step(5);
    resumed.step(5);
    assert_eq!(resumed.checkpoint_hash(), live.checkpoint_hash());
    assert_eq!(resumed.continuation().1, live.continuation().1);
}
