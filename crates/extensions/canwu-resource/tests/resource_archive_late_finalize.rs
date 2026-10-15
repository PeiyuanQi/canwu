//! The retention of an archive commit can be finalized after a later commit
//! has archived the commit's maintenance receipt.
//!
//! The state-side retention handle stays in the root until its
//! acknowledgement settles and records how the commit settled, so finalizing
//! reads the disposition from it once the receipt has left the root. The
//! fixture applies one commit, lets a second commit archive the first one's
//! receipt, and only then finalizes the first: the store-side handle is
//! finalized as committed, the acknowledgement retires the state-side
//! handle, and a restored run and an exact replay agree. A handle already
//! acknowledged, whose receipt a later commit archived, has nothing left to
//! finalize.

use canwu_api::{Canwu, EntityRef, PersonId, Scenario, SimTime};
use canwu_resource::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

#[derive(Clone, Default)]
struct MemoryArchive {
    objects: Rc<RefCell<ArchiveObjects>>,
    retention: Rc<RefCell<BTreeMap<String, ResourceArchiveRetentionPhaseV1>>>,
}

impl ResourceArchiveStore for MemoryArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .borrow_mut()
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
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .borrow_mut()
            .insert(handle.id.clone(), handle.phase);
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        *self
            .retention
            .borrow_mut()
            .get_mut(handle_id)
            .ok_or_else(|| ResourceError::NotFound("retention handle unavailable".to_owned()))? =
            phase;
        Ok(())
    }
}

impl canwu_api::PluginArchiveObjectProvider for MemoryArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_api::CanwuError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

fn state(canwu: &Canwu) -> ResourceState {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
}

fn settle(canwu: &mut Canwu) {
    canwu
        .settle_boundary(canwu_api::BoundaryRequest::at(canwu.time()))
        .expect("a resource boundary never fails in this fixture");
}

/// Archives every hot candidate in one commit and settles it, without
/// finalizing its retention.
fn archive_all(canwu: &mut Canwu, archive: &MemoryArchive) -> ResourceArchiveIngressReceiptV1 {
    let hot = state(canwu);
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), archive)
        .expect("prepare archive commit");
    let receipt = enqueue_resource_archive(canwu, &prepared, archive).expect("enqueue commit");
    settle(canwu);
    receipt
}

#[test]
fn a_commit_is_finalized_after_a_later_commit_archived_its_receipt() {
    let mut resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    for index in 0..3 {
        resource
            .apply_operation(&ResourceOperationRequestV1::CancelDemand(
                ResourceCancelDemandRequestV1 {
                    operation_key: ResourceOperationKey::new(format!("fixture:cancel:{index}"))
                        .expect("operation key"),
                    demand: ResourceDemandId::new(format!("fixture:missing:{index}"))
                        .expect("demand"),
                    expected_demand_revision: ResourceRevision::INITIAL,
                },
            ))
            .expect("durable rejection");
    }
    let plugin = ResourcePlugin::default();
    let archive = MemoryArchive::default();
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![resource.into_record().expect("resource record")]);
    let mut canwu = Canwu::new_with_plugins(4, scenario, &[&plugin]).expect("runtime");
    canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));

    // The first commit applies; the second archives its receipt before the
    // first is finalized.
    let first = archive_all(&mut canwu, &archive);
    let second = archive_all(&mut canwu, &archive);
    let both = state(&canwu);
    assert_eq!(both.archive_head.revision, 2);
    assert!(
        both.archive_maintenance_receipts
            .values()
            .all(|receipt| receipt.retention_handle_id != first.retention_handle_id)
    );
    assert_eq!(
        both.archive_retention_handles[&first.retention_handle_id].phase,
        ResourceArchiveRetentionPhaseV1::Committed
    );

    // Finalizing the first commit reads its disposition from the state-side
    // handle. Before this change the call failed, and both handles stayed
    // for the life of the run.
    finalize_resource_archive_retention(&mut canwu, &archive, &first)
        .expect("finalize a commit whose receipt a later commit archived");
    assert_eq!(
        archive.retention.borrow()[&first.retention_handle_id],
        ResourceArchiveRetentionPhaseV1::Committed
    );
    settle(&mut canwu);
    let acknowledged = state(&canwu);
    assert_eq!(
        acknowledged
            .archive_retention_handles
            .keys()
            .collect::<Vec<_>>(),
        [&second.retention_handle_id]
    );

    // The second commit is finalized from its receipt, which is still hot.
    // A third commit archives that receipt; the handle and the receipt of
    // the second are then both gone, and finalizing it again fails host-side.
    finalize_resource_archive_retention(&mut canwu, &archive, &second)
        .expect("finalize the second commit");
    settle(&mut canwu);
    assert!(state(&canwu).archive_retention_handles.is_empty());
    let third = archive_all(&mut canwu, &archive);
    assert!(finalize_resource_archive_retention(&mut canwu, &archive, &second).is_err());
    finalize_resource_archive_retention(&mut canwu, &archive, &third)
        .expect("finalize the third commit");
    settle(&mut canwu);
    assert!(state(&canwu).archive_retention_handles.is_empty());

    let snapshot = canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json_with_archive_store(&snapshot, &[&plugin], Rc::new(archive.clone()))
        .expect("restore after a late finalization");
    let replayed = replay_resource_from_journal_with_archive_store(
        &[&plugin],
        &canwu.replay_journal(),
        Rc::new(archive.clone()),
    )
    .expect("exact replay settles the late acknowledgement the same way");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
}
