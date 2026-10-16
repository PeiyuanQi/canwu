//! A run can move to another thread.
//!
//! `Canwu`, `CompactedCanwu`, and the runtimes they wrap are `Send`, and a
//! run holds its plugin archive provider as a shared `Send + Sync` object, so
//! a host may move a fork to a worker thread, for example to save it while the
//! original keeps running. The fixture's boundary system loads an archive
//! object from the attached provider and makes a keyed draw at every
//! boundary. A fork advanced on a worker thread, while the original settles
//! the same days on the test thread, must load from the shared provider there
//! and write the same receipts, full save, and continuation checkpoint as the
//! original; a compact fork must seal and write the same continuation
//! checkpoint there, and a run restored from that checkpoint must continue in
//! lockstep with the original.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryId, BoundaryPhase, BoundaryProfile,
    BoundaryProposal, BoundaryReceipt, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError,
    CheckpointJournal, CompactedCanwu, CompactedSimulation, ContinuationCheckpoint,
    DomainRecordClass, DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef,
    DomainRecordSchema, EntityRef, ErrorCode, EvidenceJournalSegment, EvidenceRef, PersonId,
    PluginArchiveObjectProvider, PluginRegistrar, RandomOperationTarget, RandomStreamKey,
    ReplayJournal, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    SystemCadence,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, ThreadId};

const PLUGIN: &str = "fixture-shelf";
const NAMESPACE: &str = "fixture.shelf";
/// How many archive objects the shelf holds; boundary `b` reads object
/// `b % SHELF_OBJECTS`.
const SHELF_OBJECTS: u64 = 7;
const SEED: u64 = 1644;

fn tally_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "tally")
}

fn tally() -> DomainRecordRef {
    DomainRecordRef {
        kind: tally_kind(),
        id: "tally".to_owned(),
    }
}

fn stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "draws", 1)
}

fn day(day: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(day)
}

fn object_id(index: u64) -> String {
    format!("item-{index}")
}

/// The host's archive: fixed objects, and the thread of every load.
struct Shelf {
    objects: BTreeMap<String, Vec<u8>>,
    loads: Mutex<Vec<ThreadId>>,
}

impl Shelf {
    fn new() -> Self {
        Self {
            objects: (0..SHELF_OBJECTS)
                .map(|index| (object_id(index), (index * 37 + 11).to_string().into_bytes()))
                .collect(),
            loads: Mutex::new(Vec::new()),
        }
    }

    fn loads_on(&self, thread: ThreadId) -> usize {
        self.loads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|load| **load == thread)
            .count()
    }
}

impl PluginArchiveObjectProvider for Shelf {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.loads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(thread::current().id());
        Ok((namespace == NAMESPACE)
            .then(|| self.objects.get(object_id).cloned())
            .flatten())
    }
}

/// Adds the boundary's archive object and a keyed draw to the tally.
fn shelve(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = context.boundary_id.get();
    let bytes = view
        .plugin_archive_object(NAMESPACE, &object_id(boundary % SHELF_OBJECTS))?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidArchive, "the shelf has no object"))?;
    let shelved: u64 = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidArchive, "the object is not a number"))?;
    let drawn = if boundary > 1 {
        let operation = format!("b{boundary}");
        view.random_draw_for_operation(
            &stream(),
            EvidenceRef::Boundary(BoundaryId::new(boundary - 1)),
            "draw",
            &operation,
            RandomOperationTarget::CanonicalKey(operation.clone()),
            0,
            1_000,
            "shelf fixture draw",
        )?
        .sample
        .value
    } else {
        0
    };
    let current = view.domain_record(&tally())?;
    let total = current
        .as_ref()
        .and_then(|record| record.payload["total"].as_u64())
        .unwrap_or_default()
        + shelved
        + drawn;
    let draft = DomainRecordDraft::new(tally(), json!({ "total": total, "boundary": boundary }));
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: match &current {
                None => DomainRecordMutation::Create { record: draft },
                Some(record) => DomainRecordMutation::Update {
                    record: draft,
                    expected_version: record.version,
                },
            },
            summary: "The tally takes the shelved object".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct ShelfFixture;

impl SimulationPlugin for ShelfFixture {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000005e4d"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            tally_kind(),
            DomainRecordClass::Record,
        ))?;
        let tally_state =
            DomainRecordSchema::new(tally_kind(), DomainRecordClass::Record).state_key();
        let mut shelving = BoundarySystemContract::new(
            "shelve",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        shelving.reads = vec![StateKey::core_evidence(), tally_state.clone()];
        shelving.writes = vec![tally_state];
        shelving.random_streams = vec![stream()];
        registrar.register_boundary_system(shelving, shelve)
    }
}

fn plugins() -> [&'static dyn SimulationPlugin; 1] {
    [&ShelfFixture]
}

fn scenario() -> Scenario {
    Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
}

fn request(d: i64) -> BoundaryRequest {
    BoundaryRequest::at(day(d)).with_cadence(SystemCadence::Daily)
}

/// A run with the shelf attached, settled through day `days`.
fn started(shelf: &Arc<Shelf>, days: i64) -> Canwu {
    let mut run = Canwu::new_with_plugins(SEED, scenario(), &plugins()).expect("fixture starts");
    run.set_plugin_archive_object_provider(shelf.clone());
    for d in 1..=days {
        run.settle_boundary(request(d)).expect("boundary settles");
    }
    run
}

/// What a run wrote after settling some days: its receipts, a save (a full
/// save, or the segment a seal returned), and its continuation checkpoint.
struct Written {
    receipts: Vec<BoundaryReceipt>,
    save: String,
    checkpoint: ContinuationCheckpoint,
    hash: String,
}

impl Written {
    fn assert_same_as(&self, other: &Self) {
        assert_eq!(self.receipts, other.receipts, "receipts");
        assert_eq!(self.save, other.save, "saves");
        assert_eq!(
            self.checkpoint.to_json().expect("JSON"),
            other.checkpoint.to_json().expect("JSON"),
            "continuation checkpoints"
        );
        assert_eq!(self.hash, other.hash, "continuation hashes");
    }
}

/// Settles days `from..=to` and writes a full save and a continuation
/// checkpoint, wherever the run is.
fn advance_and_save(run: &mut Canwu, from: i64, to: i64) -> Written {
    let receipts = (from..=to)
        .map(|d| run.settle_boundary(request(d)).expect("boundary settles"))
        .collect();
    let save = run.checkpoint_journal_json().expect("the full save");
    let (checkpoint, hash) = run
        .continuation_checkpoint()
        .expect("the continuation checkpoint builds");
    Written {
        receipts,
        save,
        checkpoint,
        hash,
    }
}

/// Settles days `from..=to`, seals, and writes a continuation checkpoint,
/// wherever the run is.
fn advance_and_seal(run: &mut CompactedCanwu, from: i64, to: i64) -> Written {
    let receipts = (from..=to)
        .map(|d| run.settle_boundary(request(d)).expect("boundary settles"))
        .collect();
    let segment: EvidenceJournalSegment = run
        .seal_evidence()
        .expect("the run seals")
        .expect("the settled tail seals");
    let (checkpoint, hash) = run
        .continuation_checkpoint()
        .expect("the continuation checkpoint builds");
    Written {
        receipts,
        save: serde_json::to_string(&segment).expect("segment JSON"),
        checkpoint,
        hash,
    }
}

const fn assert_send<T: Send>() {}

/// The attach operations take a provider the run can share across threads.
const fn attaches_shared_provider<H, P: Send + Sync + ?Sized>(_attach: fn(&mut H, Arc<P>)) {}

type Replay<P> = fn(&[&dyn SimulationPlugin], &ReplayJournal, Arc<P>) -> Result<Canwu, CanwuError>;

const fn replays_with_shared_provider<P: Send + Sync + ?Sized>(_replay: Replay<P>) {}

type ProfiledReplay<P> = fn(
    &[&dyn SimulationPlugin],
    &ReplayJournal,
    Arc<P>,
    &mut dyn FnMut(BoundaryProfile),
) -> Result<Canwu, CanwuError>;

const fn profiles_with_shared_provider<P: Send + Sync + ?Sized>(_replay: ProfiledReplay<P>) {}

#[test]
fn runtime_handles_are_send() {
    assert_send::<Canwu>();
    assert_send::<CompactedCanwu>();
    assert_send::<CompactedSimulation>();
    assert_send::<canwu_sim::Simulation>();
    // What a persistence worker hands back.
    assert_send::<CheckpointJournal>();
    assert_send::<ContinuationCheckpoint>();
    assert_send::<EvidenceJournalSegment>();
    attaches_shared_provider(Canwu::set_plugin_archive_object_provider);
    attaches_shared_provider(CompactedCanwu::set_plugin_archive_object_provider);
    attaches_shared_provider(CompactedSimulation::set_plugin_archive_object_provider);
    replays_with_shared_provider(Canwu::replay_from_journal_with_archive_provider);
    profiles_with_shared_provider(Canwu::replay_from_journal_with_boundary_profiles);
}

#[test]
fn a_fork_saved_on_another_thread_equals_the_same_steps_here() {
    let shelf = Arc::new(Shelf::new());
    let mut original = started(&shelf, 5);
    let there = original.fork();
    let here = thread::current().id();
    let loaded_here = shelf.loads_on(here);

    // The fork settles and saves on a worker while the original settles the
    // same days here, both loading from the one provider.
    let worker = thread::spawn(move || {
        let mut there = there;
        let written = advance_and_save(&mut there, 6, 15);
        (thread::current().id(), written, there)
    });
    let local = advance_and_save(&mut original, 6, 15);
    let (worker_id, remote, mut there) = worker.join().expect("the worker finishes");

    assert!(
        shelf.loads_on(worker_id) >= 10,
        "the fork loaded from the shared provider on the worker thread"
    );
    assert!(
        shelf.loads_on(here) >= loaded_here + 10,
        "the original loaded from the same provider on this thread"
    );
    remote.assert_same_as(&local);

    // The fork came back and keeps step with the original.
    original
        .settle_boundary(request(16))
        .expect("boundary settles");
    there
        .settle_boundary(request(16))
        .expect("boundary settles");
    assert_eq!(there.checkpoint_hash(), original.checkpoint_hash());
    assert_eq!(
        there.authoritative_state_hash().expect("state hash"),
        original.authoritative_state_hash().expect("state hash")
    );

    // The save written on the worker restores and continues here.
    let mut restored = Canwu::from_checkpoint_journal_json_with_plugins(&remote.save, &plugins())
        .expect("the save restores");
    restored.set_plugin_archive_object_provider(shelf.clone());
    restored
        .settle_boundary(request(16))
        .expect("boundary settles");
    assert_eq!(restored.checkpoint_hash(), original.checkpoint_hash());
}

#[test]
fn a_continuation_checkpoint_written_on_another_thread_continues_here() {
    let shelf = Arc::new(Shelf::new());
    let mut original = started(&shelf, 5)
        .into_compacted()
        .expect("the run compacts");
    original
        .seal_evidence()
        .expect("the run seals")
        .expect("the settled tail seals");
    let there = original.fork();
    let here = thread::current().id();
    let loaded_here = shelf.loads_on(here);

    // The compact fork settles, seals, and writes its checkpoint on a worker
    // while the original does the same here, both loading from the one
    // provider.
    let worker = thread::spawn(move || {
        let mut there = there;
        let written = advance_and_seal(&mut there, 6, 12);
        (thread::current().id(), written)
    });
    let local = advance_and_seal(&mut original, 6, 12);
    let (worker_id, remote) = worker.join().expect("the worker finishes");

    assert!(
        shelf.loads_on(worker_id) >= 7,
        "the compact fork loaded from the shared provider on the worker thread"
    );
    assert!(
        shelf.loads_on(here) >= loaded_here + 7,
        "the original loaded from the same provider on this thread"
    );
    remote.assert_same_as(&local);

    // The checkpoint written on the worker restores here and continues in
    // lockstep with the original.
    let mut restored = CompactedCanwu::from_continuation_checkpoint_verified(
        remote.checkpoint,
        &remote.hash,
        &plugins(),
    )
    .expect("the checkpoint restores");
    restored.set_plugin_archive_object_provider(shelf.clone());
    for d in 13..=20 {
        let expected = original
            .settle_boundary(request(d))
            .expect("boundary settles");
        let actual = restored
            .settle_boundary(request(d))
            .expect("boundary settles");
        assert_eq!(actual, expected, "receipts at day {d}");
        assert_eq!(restored.checkpoint_hash(), original.checkpoint_hash());
    }
}
