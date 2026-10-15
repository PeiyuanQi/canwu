//! An economy operation ID settles once. Admission refuses a reused ID, but
//! two commands with one ID admitted before either settles are both queued.
//! The second, whose input differs, is skipped with a visible event instead
//! of failing its boundary. A third command of the same boundary that
//! expected the revision before the first changed the root is a recorded
//! stale rejection, the root is written once with its revision one past the
//! version it started from, the run keeps settling, and a restore and an
//! exact replay of it agree.

use canwu_api::{
    CommandEnvelope, CommandRequest, CommandRequestId, Issuer, KnowledgeHolderRef, PersonId,
    SimDuration, SystemCadence, canonical_hash,
};
use canwu_economy_reference::{
    EconomyCommandV1, EconomyOperationId, EconomyOperationV1, GrainHarness, economy_command,
    economy_reference_state,
};

#[test]
fn a_reused_operation_id_with_other_input_is_skipped_and_the_run_continues() {
    let mut harness = GrainHarness::new().expect("reference composition");
    let manager = PersonId::new(1);
    let (record, state) = economy_reference_state(harness.canwu())
        .expect("economy state")
        .expect("economy runtime");
    let profile = state
        .profiles
        .values()
        .next()
        .cloned()
        .expect("a registered profile");
    let reused =
        EconomyOperationId::new("canwu.economy-reference:operation:reused-id").expect("ID");
    let other = EconomyOperationId::new("canwu.economy-reference:operation:other-id").expect("ID");
    // Registering an existing profile settles as a recorded rejection, which
    // still settles the ID and changes the root. The first two commands share
    // an ID and differ only in the expected revision.
    let command = |operation_id: &EconomyOperationId, expected_runtime_revision| EconomyCommandV1 {
        holder: KnowledgeHolderRef::Person(manager),
        operation_id: operation_id.clone(),
        expected_runtime_revision,
        operation: EconomyOperationV1::RegisterProfile {
            profile: profile.clone(),
        },
    };
    let commands = [
        command(&reused, state.revision),
        command(&reused, state.revision + 7),
        command(&other, state.revision),
    ];
    // All three are admitted in one boundary, before any settles; each
    // request names the simulation revision its admission will find.
    let canwu = harness.canwu_mut();
    let at = canwu.time();
    let revision = canwu.revision();
    for (index, command) in (0_u64..).zip(&commands) {
        canwu
            .enqueue_command(
                at,
                0,
                CommandRequest::new(
                    CommandRequestId::new(900 + index),
                    revision + index,
                    CommandEnvelope::new(
                        Issuer::Actor(manager),
                        economy_command(command).expect("command"),
                    )
                    .at_time(at),
                ),
            )
            .expect("the commands queue");
    }
    let events = canwu.events().len();
    for _ in 0..2 {
        canwu
            .advance_canonical(SimDuration::ZERO)
            .expect("the boundaries settle");
    }
    let (after, state) = economy_reference_state(canwu)
        .expect("economy state")
        .expect("economy runtime");
    assert_eq!(
        state.outcomes[&reused].input_digest,
        canonical_hash("canwu.economy.operation-input.v1", &commands[0]).expect("digest"),
        "the first command settled the ID"
    );
    assert!(
        canwu.events()[events..]
            .iter()
            .any(|event| event.summary.contains("skipped")
                && event.summary.contains(reused.as_str())),
        "the second command is skipped with a visible event"
    );
    assert_eq!(
        state.outcomes[&other].rejection_code.as_deref(),
        Some("domain_record_version_conflict"),
        "the third command expected the revision the first one changed"
    );
    assert_eq!(after.version, record.version + 1, "one write");
    assert_eq!(state.revision, after.version);

    // The run keeps settling, and a restore and an exact replay agree.
    let next = canwu.time() + SimDuration::days(1);
    canwu
        .settle_boundary(canwu_api::BoundaryRequest::at(next).with_cadence(SystemCadence::Daily))
        .expect("a later boundary settles");
    let hash = harness.canwu().checkpoint_hash().to_owned();
    let restored = GrainHarness::from_snapshot_json(&harness.snapshot_json().expect("snapshot"))
        .expect("the run restores");
    let replayed =
        GrainHarness::replay_from_journal_json(&harness.replay_journal_json().expect("journal"))
            .expect("the run replays");
    assert_eq!(restored.canwu().checkpoint_hash(), hash);
    assert_eq!(replayed.canwu().checkpoint_hash(), hash);
}
