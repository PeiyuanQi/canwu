//! Gap G-51: an operation-keyed random draw returns the ID of its draw
//! evidence with its sample, so what a system writes can cite the draw it was
//! computed from. A fresh draw returns the ID its boundary commits it under;
//! an address drawn before returns the ID of that draw, while its record is
//! retained and after it and its operation evidence are sealed; a sealed run,
//! its unsealed twin, and exact replay return the same IDs; and a failed
//! boundary leaves none behind.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, ErrorCode, EvidenceRef, IngressClass, IngressId,
    IngressPayload, PayloadSchema, PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar,
    RandomDrawAddress, RandomDrawId, RandomOperationDraw, RandomOperationTarget, RandomStreamKey,
    Scenario, SimDuration, SimEvent, SimTime, SimulationPlugin, SimulationView, StateKey,
    SystemCadence,
};
use serde_json::json;

const PLUGIN: &str = "fixture-draw-id";

fn roll_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "rolls", 1)
}

fn noise_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "noise", 1)
}

/// One admitted roll request: the operation, and the ingress record it cites
/// as operation evidence. A first request cites the packet that carries it; a
/// later request for the same operation names that first packet again, which
/// by then may be sealed.
struct Roll {
    operation: String,
    evidence: EvidenceRef,
}

fn admitted_rolls(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<Vec<Roll>, CanwuError> {
    let mut rolls = Vec::new();
    for id in &context.admitted_ingress {
        if let Some(record) = view.ingress(*id)?
            && let IngressPayload::Plugin { payload, .. } = &record.payload
            && let Some(operation) = payload["operation"].as_str()
        {
            let cited = payload["evidence"].as_u64().map_or(*id, IngressId::new);
            rolls.push(Roll {
                operation: operation.to_owned(),
                evidence: EvidenceRef::Ingress(cited),
            });
        }
    }
    Ok(rolls)
}

/// Two sequential draws ahead of the keyed ones, so a keyed draw's ID is not
/// simply its position among the keyed draws.
fn noise(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if !admitted_rolls(view, context)?.is_empty() {
        view.random_range(&noise_stream(), 6, "fixture noise")?;
        view.random_range(&noise_stream(), 6, "fixture noise")?;
    }
    Ok(BoundaryProposal::default())
}

fn roll_once(view: &SimulationView<'_>, roll: &Roll) -> Result<RandomOperationDraw, CanwuError> {
    view.random_draw_for_operation(
        &roll_stream(),
        roll.evidence.clone(),
        "roll",
        &roll.operation,
        RandomOperationTarget::CanonicalKey(roll.operation.clone()),
        0,
        1_000,
        "fixture roll",
    )
}

fn citation(operation: &str, draw: RandomDrawId, value: u64) -> String {
    format!("{operation} draw={} value={value}", draw.get())
}

/// Rolls once for each admitted request and emits an event that cites the
/// draw by ID.
fn roll(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut proposal = BoundaryProposal::default();
    for roll in admitted_rolls(view, context)? {
        let draw = roll_once(view, &roll)?;
        // An exact retry inside the boundary names the same draw, and the
        // sample-only read returns the same sample.
        let sample = view.random_sample_for_operation(
            &roll_stream(),
            roll.evidence.clone(),
            "roll",
            &roll.operation,
            RandomOperationTarget::CanonicalKey(roll.operation.clone()),
            0,
            1_000,
            "fixture roll",
        )?;
        if roll_once(view, &roll)? != draw || sample != draw.sample {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "an exact retry named a different draw",
            ));
        }
        proposal.directives.push(BoundaryDirective::Emit {
            event_type: "rolled".to_owned(),
            summary: citation(&roll.operation, draw.draw_id, draw.sample.value),
            affected: Vec::new(),
        });
    }
    Ok(proposal)
}

/// Fails every boundary that carries the Monthly cadence, after the rolls.
fn veto(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Err(CanwuError::new(
        ErrorCode::InvalidBoundary,
        "the fixture vetoes Monthly boundaries",
    ))
}

struct DrawIdPlugin;

impl SimulationPlugin for DrawIdPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a51"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "roll".to_owned(),
            description: "roll once for the named operation".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        // Systems of one phase run in name order: the noise precedes the rolls.
        let mut noisy = BoundarySystemContract::new(
            "a-noise",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        noisy.reads = vec![StateKey::core_ingress()];
        noisy.random_streams = vec![noise_stream()];
        registrar.register_boundary_system(noisy, noise)?;
        let mut rolling = BoundarySystemContract::new(
            "b-roll",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        rolling.reads = vec![StateKey::core_ingress()];
        rolling.random_streams = vec![roll_stream()];
        rolling.emits = vec!["rolled".to_owned()];
        registrar.register_boundary_system(rolling, roll)?;
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

/// A first request for `operation`, citing its own packet.
fn first_roll(at: SimTime, operation: &str) -> PluginIngressRequest {
    PluginIngressRequest::new(PLUGIN, "roll", at, json!({ "operation": operation }))
}

/// A later request for `operation`, citing the packet it was first drawn for.
fn repeat_roll(at: SimTime, operation: &str, evidence: IngressId) -> PluginIngressRequest {
    PluginIngressRequest::new(
        PLUGIN,
        "roll",
        at,
        json!({ "operation": operation, "evidence": evidence.get() }),
    )
}

fn hour(hour: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::hours(hour)
}

fn citations(events: &[SimEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind.field("event_type") == Some(json!("rolled")))
        .map(|event| event.summary.clone())
        .collect()
}

/// What an event must cite for `operation`: the ID and value of the draw
/// record the journal holds for its address.
fn journal_citation(canwu: &Canwu, operation: &str) -> String {
    let record = canwu
        .random_draws()
        .iter()
        .find(|draw| {
            matches!(
                &draw.address,
                RandomDrawAddress::OperationV1(address)
                    if address.application_operation_id == operation
            )
        })
        .expect("the journal should hold the operation's draw");
    citation(operation, record.id, record.value)
}

#[test]
fn gap_g51_sim_random_draw_id() {
    let plugin = DrawIdPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let mut canwu =
        Canwu::new_with_plugins(51, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the draw-id plugin should register");

    // A boundary that fails after the rolls commits no draw and no citation,
    // and uses up no draw ID.
    let [alpha_packet, beta_packet] = ["alpha", "beta"].map(|operation| {
        canwu
            .enqueue_plugin_ingress(first_roll(hour(0), operation))
            .expect("the roll should enqueue")
            .ingress_id
    });
    let error = canwu
        .settle_boundary(BoundaryRequest::at(hour(0)).with_cadence(SystemCadence::Monthly))
        .expect_err("the Monthly veto should fail the boundary");
    assert_eq!(error.code, ErrorCode::InvalidBoundary);
    assert!(canwu.random_draws().is_empty() && canwu.events().is_empty());

    // Fresh draws: each event cites the ID its draw was committed under,
    // behind the two sequential draws of the same boundary.
    let first = canwu
        .settle_boundary(BoundaryRequest::at(hour(0)))
        .expect("the retry should settle");
    assert_eq!(
        first.random_draws,
        [1, 2, 3, 4].map(RandomDrawId::new).to_vec()
    );
    let alpha = journal_citation(&canwu, "alpha");
    let beta = journal_citation(&canwu, "beta");
    assert!(alpha.starts_with("alpha draw=3 ") && beta.starts_with("beta draw=4 "));
    assert_eq!(citations(canwu.events()), vec![alpha.clone(), beta.clone()]);

    // A later boundary asks for `alpha` again and draws `gamma`: the retained
    // draw is named again and not drawn again; the fresh one follows the
    // boundary's two sequential draws.
    canwu
        .enqueue_plugin_ingress(repeat_roll(hour(1), "alpha", alpha_packet))
        .expect("the roll should enqueue");
    let gamma_packet = canwu
        .enqueue_plugin_ingress(first_roll(hour(1), "gamma"))
        .expect("the roll should enqueue")
        .ingress_id;
    let second = canwu
        .settle_boundary(BoundaryRequest::at(hour(1)))
        .expect("the second boundary should settle");
    assert_eq!(
        second.random_draws,
        [5, 6, 7].map(RandomDrawId::new).to_vec()
    );
    let gamma = journal_citation(&canwu, "gamma");
    assert!(gamma.starts_with("gamma draw=7 "));
    let cited = vec![alpha.clone(), beta.clone(), alpha.clone(), gamma.clone()];
    assert_eq!(citations(canwu.events()), cited);

    // Exact replay hands out the same IDs.
    let journal = serde_json::to_string(&canwu.replay_journal()).expect("journal json");
    let replayed =
        Canwu::replay_from_journal_json(&plugins, &journal).expect("the run should replay");
    assert_eq!(citations(replayed.events()), cited);
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());

    // After a seal the draw records and the packets they cite have left the
    // runtime, and each address is a keyed reservation. Asking again with the
    // sealed evidence still names the sealed draw and draws nothing, exactly
    // as in the unsealed twin. One direct boundary first admits the emitted
    // events so the evidence can be sealed.
    canwu
        .settle_boundary(BoundaryRequest::at(hour(1)))
        .expect("a direct boundary should settle");
    let mut twin = canwu.fork();
    let mut sealed_run = canwu.into_compacted().expect("compact runtime");
    let sealed = sealed_run
        .seal_evidence()
        .expect("the retained evidence should seal")
        .expect("the retained evidence is not empty");
    let repeats = [
        repeat_roll(hour(2), "gamma", gamma_packet),
        repeat_roll(hour(2), "alpha", alpha_packet),
        repeat_roll(hour(2), "beta", beta_packet),
    ];
    for request in &repeats {
        twin.enqueue_plugin_ingress(request.clone())
            .expect("the roll should enqueue");
        sealed_run
            .enqueue_plugin_ingress(request.clone())
            .expect("the roll should enqueue");
    }
    let third = sealed_run
        .settle_boundary(BoundaryRequest::at(hour(2)))
        .expect("a boundary after the seal should settle");
    assert_eq!(
        third.random_draws,
        [8, 9].map(RandomDrawId::new).to_vec(),
        "only the two sequential draws are new"
    );
    let tail = sealed_run
        .journal_segment_since(sealed.end)
        .expect("the retained tail");
    let repeated = vec![gamma, alpha, beta];
    assert_eq!(citations(&tail.events), repeated);
    let twin_third = twin
        .settle_boundary(BoundaryRequest::at(hour(2)))
        .expect("the unsealed twin should settle");
    assert_eq!(third, twin_third, "the sealed run and its twin agree");
    assert_eq!(sealed_run.checkpoint_hash(), twin.checkpoint_hash());

    // The sealed run's own journal replays exactly, with the same citations.
    let sealed_journal = sealed_run
        .replay_journal_with_segments(vec![sealed])
        .expect("the sealed run should produce its replay journal");
    let sealed_replay = Canwu::replay_from_journal(&plugins, &sealed_journal)
        .expect("the sealed run should replay");
    assert_eq!(
        sealed_replay.checkpoint_hash(),
        sealed_run.checkpoint_hash()
    );
    assert_eq!(
        citations(sealed_replay.events()),
        [cited, repeated].concat()
    );
}
