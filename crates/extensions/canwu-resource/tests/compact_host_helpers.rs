//! The resource host helpers drive a compact runtime as they drive a
//! full-history one: allocation passes, an archive commit and its retention
//! acknowledgement through an attached archive provider, and a completion
//! operation. The compact run seals its evidence after every settled step.
//! Both runs reach the same receipts, resource state, and checkpoint hashes.
//!
//! A run restored from a continuation checkpoint written with a completion
//! operation still queued continues in lockstep with the run it was written
//! from, reading the same archive.

use canwu_api::{
    BoundaryReceipt, BoundaryRequest, Canwu, CanwuError, CanwuRun, CompactedCanwu, EntityRef,
    KnowledgeHolderRef, PersonId, PluginArchiveObjectProvider, Scenario, SimDuration, SimTime,
};
use canwu_resource::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::RwLock;

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

/// A content-addressed archive store shared by the host and the runtime.
#[derive(Clone, Default)]
struct MemoryArchive {
    objects: Arc<RwLock<ArchiveObjects>>,
    retention: Arc<RwLock<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
}

impl ResourceArchiveStore for MemoryArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .write()
            .unwrap()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .write()
            .unwrap()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .write()
            .unwrap()
            .get_mut(handle_id)
            .ok_or_else(|| ResourceError::NotFound("retention handle unavailable".to_owned()))?
            .phase = phase;
        Ok(())
    }
}

impl PluginArchiveObjectProvider for MemoryArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

/// Settlement and sealing, which [`CanwuRun`] leaves out.
trait Host: CanwuRun {
    fn settle_now(&mut self) -> BoundaryReceipt;
    fn seal(&mut self);
}

impl Host for Canwu {
    fn settle_now(&mut self) -> BoundaryReceipt {
        let at = self.time();
        self.settle_boundary(BoundaryRequest::at(at))
            .expect("the boundary settles")
    }
    fn seal(&mut self) {}
}

impl Host for CompactedCanwu {
    fn settle_now(&mut self) -> BoundaryReceipt {
        let at = self.time();
        self.settle_boundary(BoundaryRequest::at(at))
            .expect("the boundary settles")
    }
    fn seal(&mut self) {
        self.seal_evidence().expect("the settled history seals");
    }
}

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

fn state(run: &(impl CanwuRun + ?Sized)) -> ResourceState {
    resource_state(run)
        .expect("resource query")
        .expect("resource state")
        .1
}

/// Settles what is due, admits its emissions, and seals.
fn step(run: &mut impl Host, receipts: &mut Vec<BoundaryReceipt>) {
    receipts.push(run.settle_now());
    receipts.push(run.settle_now());
    run.seal();
}

fn allocate(
    run: &mut impl Host,
    key: &str,
    receipts: &mut Vec<BoundaryReceipt>,
) -> ResourceAllocationRequestV1 {
    let request = ResourceAllocationRequestV1 {
        operation_key: ResourceOperationKey::new(key).expect("operation key"),
        expected_state_revision: state(run).state_revision,
        at: run.time(),
        candidate_limit: 1,
    };
    send(run, &request, receipts);
    request
}

fn send(
    run: &mut impl Host,
    request: &ResourceAllocationRequestV1,
    receipts: &mut Vec<BoundaryReceipt>,
) {
    let at = run.time();
    enqueue_resource_allocation(run, at, &holder(), request).expect("allocation enqueues");
    step(run, receipts);
}

fn drive(run: &mut impl Host, archive: &MemoryArchive) -> Vec<BoundaryReceipt> {
    let mut receipts = Vec::new();
    let first = allocate(run, "compact:allocate:a", &mut receipts);
    allocate(run, "compact:allocate:b", &mut receipts);

    // One archive commit and its acknowledgement, read back through the
    // provider the host attached to the handle.
    let hot = state(run);
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), archive)
        .expect("prepare archive commit");
    let receipt = enqueue_resource_archive(run, &prepared, archive).expect("commit enqueues");
    step(run, &mut receipts);
    finalize_resource_archive_retention(run, archive, &receipt).expect("retention finalizes");
    step(run, &mut receipts);
    assert_eq!(state(run).archive_head.revision, 1);

    let at = run.time();
    enqueue_resource_completion_expiry_tick(run, at).expect("completion expiry tick enqueues");
    step(run, &mut receipts);

    // A re-sent archived operation is a no-op, decided from the archive.
    let before = state(run);
    send(run, &first, &mut receipts);
    assert_eq!(state(run).outcomes, before.outcomes);
    receipts
}

fn runtime() -> Canwu {
    let resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource state");
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![resource.into_record().expect("resource record")]);
    Canwu::new_with_plugins(57, scenario, &[&ResourcePlugin::default()]).expect("runtime")
}

#[test]
fn resource_helpers_drive_a_sealed_compact_run_as_a_full_one() {
    let full_archive = MemoryArchive::default();
    let mut full = runtime();
    full.set_plugin_archive_object_provider(Arc::new(full_archive.clone()));
    let compact_archive = MemoryArchive::default();
    let mut compact = runtime().into_compacted().expect("compact runtime");
    compact.set_plugin_archive_object_provider(Arc::new(compact_archive.clone()));

    let full_receipts = drive(&mut full, &full_archive);
    let compact_receipts = drive(&mut compact, &compact_archive);
    assert_eq!(compact_receipts, full_receipts);
    assert_eq!(compact.checkpoint_hash(), full.checkpoint_hash());
    assert_eq!(state(&compact), state(&full));
    assert_eq!(
        *compact_archive.objects.read().unwrap(),
        *full_archive.objects.read().unwrap()
    );
    assert!(compact.boundaries().len() < full.boundaries().len());
    validate_resource_runtime_with_archive_store(&compact, &compact_archive)
        .expect("the compact run validates against its archive");
}

#[test]
fn a_continuation_restored_resource_run_continues_in_lockstep() {
    let archive = MemoryArchive::default();
    let mut full = runtime();
    full.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    let mut full_receipts = Vec::new();
    let first = allocate(&mut full, "continuation:allocate:a", &mut full_receipts);
    allocate(&mut full, "continuation:allocate:b", &mut full_receipts);
    let hot = state(&full);
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), &archive)
        .expect("prepare archive commit");
    let receipt = enqueue_resource_archive(&mut full, &prepared, &archive).expect("commit");
    step(&mut full, &mut full_receipts);
    finalize_resource_archive_retention(&mut full, &archive, &receipt).expect("retention");
    step(&mut full, &mut full_receipts);

    // A completion expiry tick is queued for a later instant, so it is
    // pending at the cut.
    let at = full.time() + SimDuration::minutes(5);
    enqueue_resource_completion_expiry_tick(&mut full, at)
        .expect("completion expiry tick enqueues");
    let (checkpoint, hash) = full
        .continuation_checkpoint()
        .expect("the continuation checkpoint builds");
    assert_eq!(checkpoint.pending_ingress.len(), 1);
    let mut restored = CompactedCanwu::from_continuation_checkpoint_verified(
        checkpoint,
        &hash,
        &[&ResourcePlugin::default()],
    )
    .expect("the resource run restores");
    restored.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    assert_eq!(restored.checkpoint_hash(), full.checkpoint_hash());

    full_receipts.extend(full.settle_at(at));
    let mut restored_receipts = restored.settle_at(at);
    // A re-sent archived operation is a no-op, decided from the archive, in
    // both runs.
    send(&mut full, &first, &mut full_receipts);
    send(&mut restored, &first, &mut restored_receipts);
    for _ in 0..30 {
        step(&mut full, &mut full_receipts);
        step(&mut restored, &mut restored_receipts);
        assert_eq!(restored.checkpoint_hash(), full.checkpoint_hash());
    }
    assert_eq!(
        restored_receipts,
        full_receipts[full_receipts.len() - restored_receipts.len()..]
    );
    assert_eq!(state(&restored), state(&full));
    assert_eq!(
        restored.authoritative_state_hash().expect("state hash"),
        full.authoritative_state_hash().expect("state hash")
    );
    validate_resource_runtime_with_archive_store(&restored, &archive)
        .expect("the restored run validates against the archive");
}

/// Settles the boundary at an instant and the one that admits its
/// emissions.
trait Settle {
    fn settle_at(&mut self, at: SimTime) -> Vec<BoundaryReceipt>;
}

impl Settle for Canwu {
    fn settle_at(&mut self, at: SimTime) -> Vec<BoundaryReceipt> {
        let first = self
            .settle_boundary(BoundaryRequest::at(at))
            .expect("the boundary settles");
        vec![first, self.settle_now()]
    }
}

impl Settle for CompactedCanwu {
    fn settle_at(&mut self, at: SimTime) -> Vec<BoundaryReceipt> {
        let first = self
            .settle_boundary(BoundaryRequest::at(at))
            .expect("the boundary settles");
        let receipts = vec![first, self.settle_now()];
        self.seal();
        receipts
    }
}
