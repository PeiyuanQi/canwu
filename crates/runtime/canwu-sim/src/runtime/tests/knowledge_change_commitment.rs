//! Every field of a boundary's knowledge changes is covered by the boundary
//! hash (commitment format 5), so an edit that load validation would accept
//! on its own is refused unless the boundary chain is rehashed.

use super::*;

/// A snapshot whose first boundary publishes three knowledge changes: a
/// phase-4 same-boundary record, a phase-4 next-boundary record, and a
/// phase-13 same-boundary record that relates to neither.
fn published() -> (SimulationSnapshot, KnowledgePublicationPlugin) {
    let (scenario, _) = demo_scenario();
    let plugin = KnowledgePublicationPlugin;
    let mut simulation =
        Simulation::new(67, scenario).expect("the knowledge commitment fixture should load");
    simulation
        .register_plugin(&plugin)
        .expect("the publication plugin should register");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("the publications should settle");
    (simulation.snapshot(), plugin)
}

fn load(
    snapshot: SimulationSnapshot,
    plugin: &KnowledgePublicationPlugin,
) -> Result<(), CanwuError> {
    Simulation::from_snapshot_with_plugins(snapshot, &[plugin]).map(|_| ())
}

/// Refused when the edit is all that changed, and when the commitments and
/// checkpoint are refreshed around a boundary chain that was not rehashed.
/// Accepted once the chain is rehashed too, which shows that the boundary
/// hash, not another load check, is what refuses the first two.
fn assert_covered(label: &str, edit: impl Fn(&mut BoundaryKnowledgeChange)) {
    let (snapshot, plugin) = published();
    let tamper = || {
        let mut tampered = snapshot.clone();
        edit(&mut tampered.boundaries[0].knowledge_changes[0]);
        assert_ne!(
            tampered, snapshot,
            "{label}: the edit should change the snapshot"
        );
        tampered
    };

    let error = load(tamper(), &plugin)
        .expect_err("an edited knowledge change must not load without a rehash");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
    assert!(
        error.message.contains("boundary hash"),
        "{label}: {}",
        error.message
    );

    let mut refreshed = tamper();
    refresh_snapshot_commitments_and_checkpoint(&mut refreshed);
    let error = load(refreshed, &plugin)
        .expect_err("refreshed commitments must not cover an unhashed boundary edit");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
    assert!(
        error.message.contains("boundary hash"),
        "{label}: {}",
        error.message
    );

    let mut rehashed = tamper();
    rehash_tampered_snapshot(&mut rehashed);
    load(rehashed, &plugin)
        .unwrap_or_else(|error| panic!("{label}: a coherently rehashed edit should load: {error}"));
}

#[test]
fn boundary_hash_covers_knowledge_change_visibility_and_correlation() {
    let (snapshot, plugin) = published();
    assert_eq!(
        snapshot.commitment_format_version,
        COMMITMENT_FORMAT_VERSION
    );
    assert_eq!(COMMITMENT_FORMAT_VERSION, 5);
    let first = &snapshot.boundaries[0].knowledge_changes[0];
    assert_eq!(first.phase, BoundaryPhase::PerceptionAndAttentionRefresh);
    assert_eq!(first.visibility, StateVisibility::SameBoundary);
    assert!(first.producer_correlation.is_some());

    // The holder views count a next-boundary publication from the end of
    // its boundary, so this flip moves what a holder may see.
    assert_covered("visibility", |change| {
        change.visibility = StateVisibility::NextBoundary;
    });
    assert_covered("producer correlation", |change| {
        change.producer_correlation = Some("forged-correlation".to_owned());
    });
    assert_covered("absent producer correlation", |change| {
        change.producer_correlation = None;
    });

    // A snapshot that declares the previous commitment format is refused.
    let mut previous_format = snapshot;
    previous_format.commitment_format_version = 4;
    let error = load(previous_format, &plugin).expect_err("a format 4 snapshot must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}
