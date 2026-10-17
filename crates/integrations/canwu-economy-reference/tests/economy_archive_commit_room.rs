//! A plugin root refuses more retention handles and maintenance receipts than
//! its bounds, and a boundary that settled an archive commit past them would
//! fail for good. The enqueue helper therefore refuses, on the host, a commit
//! that the root and the commits already queued leave no room for, and the
//! commits it did queue settle.

use canwu_api::{
    CommandEnvelope, CommandRequest, CommandRequestId, Issuer, KnowledgeHolderRef, PersonId,
    SimDuration, SystemCadence,
};
use canwu_economy_reference::{
    EconomyCommandV1, EconomyOperationId, EconomyOperationV1, GrainHarness, economy_command,
    economy_reference_state, enqueue_economy_archive,
};
use canwu_force_supply_reference::MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES;

#[test]
fn an_archive_commit_the_root_has_no_room_for_is_refused_before_it_is_queued() {
    let mut harness = GrainHarness::new().expect("reference composition");
    let manager = PersonId::new(1);
    let (_, state) = economy_reference_state(harness.canwu())
        .expect("economy state")
        .expect("economy runtime");
    // One settled operation outcome to archive: registering an existing
    // profile is a recorded rejection.
    let command = EconomyCommandV1 {
        holder: KnowledgeHolderRef::Person(manager),
        operation_id: EconomyOperationId::new("canwu.economy-reference:operation:archived")
            .expect("ID"),
        expected_runtime_revision: state.revision,
        operation: EconomyOperationV1::RegisterProfile {
            profile: state.profiles.values().next().cloned().expect("a profile"),
        },
    };
    let canwu = harness.canwu_mut();
    let at = canwu.time();
    canwu
        .enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(900),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(manager),
                    economy_command(&command).expect("command"),
                )
                .at_time(at),
            ),
        )
        .expect("the command queues");
    for _ in 0..2 {
        canwu
            .advance_canonical(SimDuration::ZERO)
            .expect("the boundaries settle");
    }

    // The same batch may be queued again and again; each copy settles, the
    // first as applied and the rest as stale. The root holds no handle yet,
    // so as many copies as it may hold handles are queued, and the next is
    // refused with nothing queued.
    let store = harness.archive_store();
    let (_, state) = economy_reference_state(harness.canwu())
        .expect("economy state")
        .expect("economy runtime");
    let force = harness
        .canwu()
        .typed_domain_record(&canwu_force_supply_reference::force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<canwu_force_supply_reference::ForceSupplyRuntimeRecord>()
        .expect("force state");
    let prepared = state
        .prepare_economy_archive(8, store.as_ref(), Some(&force))
        .expect("the batch prepares");
    for _ in 0..MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES {
        enqueue_economy_archive(harness.canwu_mut(), &prepared, store.as_ref())
            .expect("the commit queues");
    }
    let error = enqueue_economy_archive(harness.canwu_mut(), &prepared, store.as_ref())
        .expect_err("no room is left for another commit");
    assert!(error.message.starts_with("archive_backpressure"));

    // The queued commits settle, and the run goes on.
    let canwu = harness.canwu_mut();
    canwu
        .advance_canonical(SimDuration::ZERO)
        .expect("the queued commits settle");
    let next = canwu.time() + SimDuration::days(1);
    canwu
        .settle_boundary(canwu_api::BoundaryRequest::at(next).with_cadence(SystemCadence::Daily))
        .expect("a later boundary settles");
    let (_, state) = economy_reference_state(harness.canwu())
        .expect("economy state")
        .expect("economy runtime");
    assert_eq!(
        state.archive_maintenance_receipts.len(),
        MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES
    );
    assert_eq!(state.archive_head.revision, 1, "one copy applied");
}
