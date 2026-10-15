use super::*;

const INDEX_PLUGIN: &str = "fixture-keyed-index";

fn index_stream() -> RandomStreamKey {
    RandomStreamKey::new(INDEX_PLUGIN, "resolution", 1)
}

/// Draws one keyed sample for each admitted packet that names an operation,
/// after one sequential draw that shifts the boundary's draw IDs. A packet
/// that names none makes the system due without a keyed draw.
fn draw_admitted_operations(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let _ = view.random_range(&index_stream(), 6, "sequential control draw")?;
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin { payload, .. } = &record.payload else {
            continue;
        };
        let Some(operation) = payload["operation"].as_str() else {
            continue;
        };
        view.random_range_for_operation(
            &index_stream(),
            EvidenceRef::Ingress(*id),
            "resolve",
            operation,
            RandomOperationTarget::CanonicalKey(operation.to_owned()),
            0,
            10_000,
            "indexed resolution",
        )?;
    }
    Ok(BoundaryProposal::default())
}

struct KeyedIndexPlugin;

impl SimulationPlugin for KeyedIndexPlugin {
    fn name(&self) -> &'static str {
        INDEX_PLUGIN
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000a50");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "operation".to_owned(),
            description: "Admit one keyed random operation".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut draw = BoundarySystemContract::new(
            "draw",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        draw.reads = vec![StateKey::core_ingress()];
        draw.random_streams = vec![index_stream()];
        registrar.register_boundary_system(draw, draw_admitted_operations)
    }
}

fn enqueue_operations(simulation: &mut Simulation, at: SimTime, operations: &[&str]) {
    for operation in operations {
        simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                INDEX_PLUGIN,
                "operation",
                at,
                serde_json::json!({ "operation": operation }),
            ))
            .expect("operation ingress should enqueue");
    }
}

/// The keyed-draw ledger a runtime keeps, after checking that it equals the
/// ledger built from its complete draw journal: the draws `sealed` before its
/// retained tail, followed by the retained draws.
fn exact_ledger(simulation: &Simulation, sealed: &[RandomDrawRecord]) -> random::KeyedDrawLedger {
    let evidence = &simulation.state.evidence;
    let journal = sealed
        .iter()
        .chain(&evidence.random_draws)
        .cloned()
        .collect::<Vec<_>>();
    let rebuilt =
        random::keyed_draw_ledger(&journal).expect("the complete draw journal should index");
    assert_eq!(evidence.keyed_draws, rebuilt);
    rebuilt
}

fn exact_index(simulation: &Simulation) -> random::KeyedDrawLedger {
    exact_ledger(simulation, &[])
}

/// An in-memory state-page store for the paged restore.
#[derive(Default)]
struct MemoryStatePages(std::cell::RefCell<BTreeMap<String, StatePageBlob>>);

impl StatePageProvider for MemoryStatePages {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(self.0.borrow().get(page_id).cloned())
    }
}

impl StatePageStore for MemoryStatePages {
    fn store_state_page(&self, page: &StatePageBlob) -> Result<ArchiveStoreOutcome, CanwuError> {
        Ok(
            match self
                .0
                .borrow_mut()
                .insert(page.page_id.clone(), page.clone())
            {
                Some(_) => ArchiveStoreOutcome::AlreadyPresent,
                None => ArchiveStoreOutcome::Stored,
            },
        )
    }
}

/// How many keyed-draw ledgers `operation` builds from a draw journal.
fn ledger_builds<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    let before = random::LEDGER_BUILDS.with(std::cell::Cell::get);
    let value = operation();
    let after = random::LEDGER_BUILDS.with(std::cell::Cell::get);
    (value, after - before)
}

#[test]
fn keyed_draw_index_stays_exact_through_commit_rollback_seal_restore_replay_and_fork() {
    let plugin = KeyedIndexPlugin;
    let (scenario, _) = demo_scenario();
    let mut simulation =
        Simulation::new(150, scenario).expect("the keyed index fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("the keyed index plugin should register");
    assert_eq!(exact_index(&simulation).len(), 0);

    // Commit: two keyed draws behind one sequential draw.
    enqueue_operations(&mut simulation, SimTime::EPOCH, &["alpha", "beta"]);
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the first boundary should settle");
    let committed = exact_index(&simulation);
    assert_eq!(committed.len(), 2);

    // A boundary that fails after it appended its draws to the journal and
    // installed its index (here the state revision is exhausted) leaves the
    // index, the journal, and the draw counter as they were, and the retry
    // draws the address the failed boundary had drawn.
    enqueue_operations(&mut simulation, SimTime::EPOCH, &["gamma"]);
    let before_failure = simulation.snapshot();
    let revision = simulation.state.counters.state_revision;
    simulation.state.counters.state_revision = u64::MAX;
    let error = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect_err("an exhausted state revision should fail the boundary");
    assert_eq!(error.code, ErrorCode::IdentifierExhausted);
    simulation.state.counters.state_revision = revision;
    assert_eq!(simulation.snapshot(), before_failure);
    assert_eq!(exact_index(&simulation), committed);
    let fork = simulation.fork();
    let retry = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the retry should settle");
    assert_eq!(retry.random_draws.len(), 2);
    let live = exact_index(&simulation);
    assert_eq!(live.len(), 3);
    // The fork shares the index it was taken with and then diverges.
    assert_eq!(exact_index(&fork), committed);

    // Save/load, a checkpoint-journal restore, a paged restore, and exact
    // replay rebuild the same ledger, each restore building it once.
    let snapshot = simulation.snapshot();
    let (restored, builds) =
        ledger_builds(|| Simulation::from_snapshot_with_plugins(snapshot, &[&plugin]));
    let restored = restored.expect("the run should load");
    assert_eq!(builds, 1, "a snapshot restore builds the ledger once");
    assert_eq!(exact_index(&restored), live);
    let bundle = simulation
        .checkpoint_journal()
        .expect("a checkpoint journal");
    let (from_bundle, builds) =
        ledger_builds(|| Simulation::from_checkpoint_journal_with_plugins(bundle, &[&plugin]));
    let from_bundle = from_bundle.expect("the checkpoint journal should restore");
    assert_eq!(
        builds, 1,
        "a checkpoint-journal restore builds the ledger once"
    );
    assert_eq!(exact_index(&from_bundle), live);
    let pages = MemoryStatePages::default();
    let prepared = simulation
        .prepare_paged_checkpoint(None, &pages)
        .expect("a paged checkpoint");
    prepared
        .store_and_verify(&pages)
        .expect("the paged checkpoint should store");
    let segment = simulation
        .journal_segment_since(EvidenceCursor::default())
        .expect("the evidence prefix");
    let (from_pages, builds) = ledger_builds(|| {
        Simulation::from_paged_checkpoint_and_journal_with_plugins(
            &prepared.checkpoint,
            &pages,
            vec![segment],
            &[&plugin],
        )
    });
    let from_pages = from_pages.expect("the paged checkpoint should restore");
    assert_eq!(builds, 1, "a paged restore builds the ledger once");
    assert_eq!(exact_index(&from_pages), live);
    let (replayed, builds) =
        ledger_builds(|| Simulation::replay_from_journal(&[&plugin], &simulation.replay_journal()));
    let replayed = replayed.expect("the run should replay");
    assert!(builds <= 1, "exact replay builds the ledger at most once");
    assert_eq!(exact_index(&replayed), live);

    // A seal, a checkpoint, and a paged checkpoint leave the ledger as it
    // was: the sealed draws stay in it and nothing copies it.
    let mut compact = simulation
        .into_compacted()
        .expect("the run should enter the compact runtime");
    let kept = compact.runtime().state.evidence.keyed_draws.clone();
    let sealed = compact
        .seal_evidence()
        .expect("the retained evidence should seal")
        .expect("the retained evidence is not empty");
    assert!(compact.runtime().state.evidence.random_draws.is_empty());
    compact.checkpoint().expect("a compact checkpoint");
    assert!(
        compact
            .runtime()
            .state
            .evidence
            .keyed_draws
            .shares_root_with(&kept)
    );
    assert_eq!(exact_ledger(compact.runtime(), &sealed.random_draws), live);
    compact
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            INDEX_PLUGIN,
            "operation",
            SimTime::EPOCH,
            serde_json::json!({ "operation": "delta" }),
        ))
        .expect("operation ingress should enqueue");
    compact
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("a boundary after the seal should settle");
    let continued = exact_ledger(compact.runtime(), &sealed.random_draws);
    assert_eq!(continued.len(), 4);

    // A restore from the sealed checkpoint retains every draw again; the
    // index it rebuilds from draw records equals the one kept across the seal.
    let tail = compact
        .journal_segment_since(sealed.end)
        .expect("the retained tail should materialize");
    let (resumed, builds) = ledger_builds(|| {
        Simulation::from_checkpoint_and_journal(
            compact.checkpoint().expect("compact checkpoint"),
            vec![sealed, tail],
        )
    });
    let resumed = resumed.expect("the sealed run should restore");
    assert_eq!(
        builds, 1,
        "a restore from sealed segments builds the ledger once"
    );
    assert_eq!(exact_index(&resumed), continued);
}

/// The kept index is neither rebuilt nor copied when a boundary settles: a
/// boundary whose due system makes no keyed draw leaves the very tree that
/// was there before, a fork holds that tree too, and a new address replaces
/// the tree only in the runtime that drew it.
#[test]
fn a_boundary_without_keyed_draws_and_a_fork_share_the_kept_index() {
    let plugin = KeyedIndexPlugin;
    let (scenario, _) = demo_scenario();
    let mut simulation =
        Simulation::new(152, scenario).expect("the keyed index fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("the keyed index plugin should register");
    enqueue_operations(&mut simulation, SimTime::EPOCH, &["alpha", "beta"]);
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the first boundary should settle");
    let before = simulation.state.evidence.keyed_draws.clone();
    assert_eq!(before.len(), 2);

    simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            INDEX_PLUGIN,
            "operation",
            SimTime::EPOCH,
            serde_json::json!({}),
        ))
        .expect("an idle packet should enqueue");
    let idle = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the idle boundary should settle");
    assert_eq!(idle.random_draws.len(), 1, "the system ran and drew once");
    assert!(
        simulation
            .state
            .evidence
            .keyed_draws
            .shares_root_with(&before)
    );
    let fork = simulation.fork();
    assert!(fork.state.evidence.keyed_draws.shares_root_with(&before));

    enqueue_operations(&mut simulation, SimTime::EPOCH, &["gamma"]);
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the drawing boundary should settle");
    assert!(
        !simulation
            .state
            .evidence
            .keyed_draws
            .shares_root_with(&before)
    );
    assert!(fork.state.evidence.keyed_draws.shares_root_with(&before));
    assert_eq!(exact_index(&fork), before);
}
