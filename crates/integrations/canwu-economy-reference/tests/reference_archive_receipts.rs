//! The reference package archive keeps a bounded hot set, and an excess is
//! refused, never a failed boundary.
//!
//! - The acknowledgement of a commit's retention folds every receipt
//!   recorded under its handle into the root's receipt chain, so the root
//!   holds no receipt of an acknowledged commit; a repeated acknowledgement
//!   settles as a no-op, and receipt sequences stay unique.
//! - An operation whose outcome finds no hot slot, or whose result passes
//!   the share of the byte cap operations may fill, is skipped with an
//!   `operation_skipped` event, records nothing, and settles when sent again
//!   after an archive commit; one whose result passes another bounded
//!   collection is a recorded rejection. Archive maintenance still settles
//!   in the reserve operations leave free.
//! - An economy commit leaves out an externality outcome the force-supply
//!   root still holds hot, whose commit would fail its boundary.

use canwu_api::{
    CommandEnvelope, CommandRequest, CommandRequestId, Issuer, KnowledgeHolderRef, PersonId,
    SimDuration, SystemCadence,
};
use canwu_economy_reference::{
    ECONOMY_ARCHIVE_DOMAIN, EconomyArchiveKeyV1, EconomyArchivePayloadV1, EconomyCommandV1,
    EconomyOperationId, EconomyOperationV1, EconomyProfileId, EconomyReferenceLimitsV1,
    EconomyReferenceStateV1, GrainDecision, GrainHarness, economy_command,
    economy_operation_outcome, economy_operation_skip, economy_reference_state,
    enqueue_economy_archive, finalize_economy_archive_retention,
};
use canwu_force_supply_reference::{
    ForceCommandEnvelopeV1, ForceOperationId, ForceOperationV1, ForceSupplyReferenceLimitsV1,
    ForceSupplyRuntimeRecord, ForceSupplyStateV1, MIN_PACKAGE_STATE_BYTES,
    PackageArchiveReceiptChainV1, PackageArchiveRetentionPhaseV1, PackageRootShortageV1,
    RequisitionSagaId, enqueue_force_archive, finalize_force_archive_retention,
    force_operation_outcome, force_operation_skip, force_supply_command,
    force_supply_runtime_reference, load_package_archive_record, package_operation_state_bytes,
};

const MANAGER: PersonId = PersonId::new(1);
const FORCE_HOLDER: PersonId = PersonId::new(2);

fn economy(harness: &GrainHarness) -> EconomyReferenceStateV1 {
    economy_reference_state(harness.canwu())
        .expect("economy state")
        .expect("economy runtime")
        .1
}

fn force(harness: &GrainHarness) -> ForceSupplyStateV1 {
    harness
        .canwu()
        .typed_domain_record(&force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state")
}

/// Queues `commands` as tracked commands of one boundary, each naming the
/// simulation revision its admission finds, and settles them.
fn settle_commands(
    harness: &mut GrainHarness,
    first_request: u64,
    commands: &[(PersonId, canwu_api::Command)],
) {
    let canwu = harness.canwu_mut();
    let at = canwu.time();
    let revision = canwu.revision();
    for (index, (actor, command)) in (0_u64..).zip(commands) {
        canwu
            .enqueue_command(
                at,
                0,
                CommandRequest::new(
                    CommandRequestId::new(first_request + index),
                    revision + index,
                    CommandEnvelope::new(Issuer::Actor(*actor), command.clone()).at_time(at),
                ),
            )
            .expect("the command queues");
    }
    for _ in 0..2 {
        canwu
            .advance_canonical(SimDuration::ZERO)
            .expect("the boundaries settle");
    }
}

/// An economy command that registers the run's own profile again, a
/// recorded rejection whose outcome is an archive candidate.
fn rejected_economy_command(harness: &GrainHarness, id: &str) -> (PersonId, canwu_api::Command) {
    let state = economy(harness);
    let command = EconomyCommandV1 {
        holder: KnowledgeHolderRef::Person(MANAGER),
        operation_id: EconomyOperationId::new(format!("canwu.economy-reference:operation:{id}"))
            .expect("ID"),
        expected_runtime_revision: state.revision,
        operation: EconomyOperationV1::RegisterProfile {
            profile: state.profiles.values().next().cloned().expect("a profile"),
        },
    };
    (MANAGER, economy_command(&command).expect("command"))
}

/// A force-supply command whose expected revision is stale, a recorded
/// rejection whose outcome is an archive candidate.
fn rejected_force_command(harness: &GrainHarness, id: &str) -> (PersonId, canwu_api::Command) {
    let command = ForceCommandEnvelopeV1 {
        operation_id: ForceOperationId::new(format!("canwu.force-supply-reference:operation:{id}"))
            .expect("ID"),
        holder: KnowledgeHolderRef::Person(FORCE_HOLDER),
        expected_runtime_revision: force(harness).revision + 1_000,
        operation: ForceOperationV1::FinalizeRequisition {
            saga: RequisitionSagaId::new("canwu.force-supply-reference:saga:never-opened")
                .expect("ID"),
        },
    };
    (
        FORCE_HOLDER,
        force_supply_command(&command).expect("command"),
    )
}

fn settle_now(harness: &mut GrainHarness) {
    harness
        .canwu_mut()
        .advance_canonical(SimDuration::ZERO)
        .expect("the boundary settles");
}

fn assert_restore_and_replay_agree(harness: &mut GrainHarness) {
    let canwu = harness.canwu_mut();
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

#[test]
fn acknowledging_a_commit_folds_its_receipts_out_of_the_root() {
    let mut harness = GrainHarness::new().expect("reference composition");
    let store = harness.archive_store();
    let command = rejected_economy_command(&harness, "fold-first");
    settle_commands(&mut harness, 900, &[command]);

    // The same batch three times: one copy applies, two settle stale, and
    // all three record their receipts under one retention handle.
    let prepared = economy(&harness)
        .prepare_economy_archive(8, store.as_ref(), Some(&force(&harness)))
        .expect("the batch prepares");
    let mut queued = Vec::new();
    for _ in 0..3 {
        queued.push(
            enqueue_economy_archive(harness.canwu_mut(), &prepared, store.as_ref())
                .expect("the commit queues"),
        );
    }
    assert!(
        finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), &queued[0])
            .is_err(),
        "a commit that has not settled has nothing to finalize"
    );
    settle_now(&mut harness);
    let state = economy(&harness);
    assert_eq!(state.archive_maintenance_receipts.len(), 3);
    assert_eq!(state.archive_retention_handles.len(), 1);
    assert_eq!(state.archive_receipt_chain.issued, 3);
    let receipts = state
        .archive_maintenance_receipts
        .values()
        .cloned()
        .collect::<Vec<_>>();

    // Finalized twice before either acknowledgement settles: the first
    // folds all three receipts, the second finds nothing and settles.
    for receipt in &queued[..2] {
        finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), receipt)
            .expect("the acknowledgement queues")
            .expect("an acknowledgement");
    }
    settle_now(&mut harness);
    let state = economy(&harness);
    assert!(state.archive_maintenance_receipts.is_empty());
    assert!(state.archive_retention_handles.is_empty());
    let mut expected = PackageArchiveReceiptChainV1 {
        issued: 3,
        ..PackageArchiveReceiptChainV1::default()
    };
    for receipt in &receipts {
        expected
            .fold(ECONOMY_ARCHIVE_DOMAIN, receipt)
            .expect("fold");
    }
    assert_eq!(state.archive_receipt_chain, expected);
    assert_eq!(
        store
            .load_package_archive_retention(&queued[0].retention_handle_id)
            .expect("store")
            .expect("handle")
            .phase,
        PackageArchiveRetentionPhaseV1::Committed,
        "the applied copy decides the phase"
    );
    assert!(
        finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), &queued[2])
            .expect("a finalization retried after its acknowledgement")
            .is_none(),
        "nothing is left to finalize once the acknowledgement settled"
    );

    // The next commit's receipt continues the sequence and folds in turn.
    let command = rejected_economy_command(&harness, "fold-second");
    settle_commands(&mut harness, 910, &[command]);
    let prepared = economy(&harness)
        .prepare_economy_archive(8, store.as_ref(), Some(&force(&harness)))
        .expect("the batch prepares");
    let next = enqueue_economy_archive(harness.canwu_mut(), &prepared, store.as_ref())
        .expect("the commit queues");
    settle_now(&mut harness);
    let state = economy(&harness);
    assert_eq!(
        state
            .archive_maintenance_receipts
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [4]
    );
    expected.issued = 4;
    expected
        .fold(
            ECONOMY_ARCHIVE_DOMAIN,
            &state.archive_maintenance_receipts[&4],
        )
        .expect("fold");
    finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), &next)
        .expect("the acknowledgement queues");
    settle_now(&mut harness);
    let state = economy(&harness);
    assert!(state.archive_maintenance_receipts.is_empty());
    assert_eq!(state.archive_receipt_chain, expected);
    assert_eq!(state.archive_head.revision, 2);

    assert_restore_and_replay_agree(&mut harness);
}

#[test]
#[allow(clippy::too_many_lines)]
fn an_operation_without_room_is_skipped_and_settles_when_sent_again() {
    let mut harness = GrainHarness::with_limits(
        EconomyReferenceLimitsV1 {
            max_operation_outcomes: 2,
            max_profiles: 1,
            ..EconomyReferenceLimitsV1::canonical()
        },
        ForceSupplyReferenceLimitsV1 {
            max_externality_outcomes: 1,
            ..ForceSupplyReferenceLimitsV1::canonical()
        },
    )
    .expect("reference composition");
    let store = harness.archive_store();
    let id = |name: &str| {
        EconomyOperationId::new(format!("canwu.economy-reference:operation:{name}")).expect("ID")
    };
    let force_id = |name: &str| {
        ForceOperationId::new(format!("canwu.force-supply-reference:operation:{name}")).expect("ID")
    };

    // A second profile passes the profile bound: a recorded rejection. The
    // next command is stale and fills the second outcome slot; the third
    // finds no slot and is skipped. The force-supply root has one slot.
    let state = economy(&harness);
    let mut second = state.profiles.values().next().cloned().expect("a profile");
    second.id = EconomyProfileId::new("canwu.economy-reference:profile:second").expect("ID");
    let second = second.seal().expect("profile");
    let register = EconomyCommandV1 {
        holder: KnowledgeHolderRef::Person(MANAGER),
        operation_id: id("second-profile"),
        expected_runtime_revision: state.revision,
        operation: EconomyOperationV1::RegisterProfile { profile: second },
    };
    let commands = [
        (MANAGER, economy_command(&register).expect("command")),
        rejected_economy_command(&harness, "stale"),
        rejected_economy_command(&harness, "skipped"),
        rejected_force_command(&harness, "first"),
        rejected_force_command(&harness, "skipped"),
    ];
    let events = harness.canwu().events().len();
    settle_commands(&mut harness, 900, &commands);
    let state = economy(&harness);
    assert_eq!(
        state.outcomes[&id("second-profile")]
            .rejection_code
            .as_deref(),
        Some("query_budget_exceeded")
    );
    assert_eq!(state.profiles.len(), 1);
    assert!(state.outcomes.contains_key(&id("stale")));
    assert!(!state.outcomes.contains_key(&id("skipped")));
    let force_state = force(&harness);
    assert!(force_state.outcomes.contains_key(&force_id("first")));
    assert!(!force_state.outcomes.contains_key(&force_id("skipped")));
    // Each skip is its own event type, read back with its reason; no
    // rejection event names a skipped operation.
    let new_events = &harness.canwu().events()[events..];
    let economy_skips = new_events
        .iter()
        .filter_map(economy_operation_skip)
        .collect::<Vec<_>>();
    let force_skips = new_events
        .iter()
        .filter_map(force_operation_skip)
        .collect::<Vec<_>>();
    assert_eq!(economy_skips.len(), 1);
    assert_eq!(economy_skips[0].operation_id, id("skipped").as_str());
    assert_eq!(
        economy_skips[0].shortage,
        PackageRootShortageV1::ArchiveBacklog
    );
    assert_eq!(force_skips.len(), 1);
    assert_eq!(force_skips[0].operation_id, force_id("skipped").as_str());
    assert_eq!(
        force_skips[0].shortage,
        PackageRootShortageV1::ArchiveBacklog
    );
    assert!(!new_events.iter().any(|event| {
        event.summary.contains("rejected")
            && (event.summary.contains(id("skipped").as_str())
                || event.summary.contains(force_id("skipped").as_str()))
    }));

    // Archive both roots and acknowledge; the outcomes stay readable.
    let prepared = force_state
        .prepare_force_archive(8, store.as_ref())
        .expect("the force batch prepares");
    let receipt = enqueue_force_archive(harness.canwu_mut(), &prepared, store.as_ref())
        .expect("the force commit queues");
    settle_now(&mut harness);
    finalize_force_archive_retention(harness.canwu_mut(), store.as_ref(), &receipt)
        .expect("the force acknowledgement queues");
    settle_now(&mut harness);
    let prepared = economy(&harness)
        .prepare_economy_archive(8, store.as_ref(), Some(&force(&harness)))
        .expect("the economy batch prepares");
    let receipt = enqueue_economy_archive(harness.canwu_mut(), &prepared, store.as_ref())
        .expect("the economy commit queues");
    settle_now(&mut harness);
    finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), &receipt)
        .expect("the economy acknowledgement queues");
    settle_now(&mut harness);
    let state = economy(&harness);
    let force_state = force(&harness);
    assert!(state.outcomes.is_empty());
    assert!(force_state.outcomes.is_empty());
    assert!(state.archive_maintenance_receipts.is_empty());
    assert!(force_state.archive_maintenance_receipts.is_empty());
    assert_eq!(force_state.archive_receipt_chain.folded, 1);
    assert!(
        economy_operation_outcome(&state, harness.canwu(), &id("stale"))
            .expect("lookup")
            .is_some_and(|outcome| !outcome.applied)
    );
    assert!(
        force_operation_outcome(&force_state, harness.canwu(), &force_id("first"))
            .expect("lookup")
            .is_some()
    );
    assert!(
        economy_operation_outcome(&state, harness.canwu(), &id("skipped"))
            .expect("lookup")
            .is_none()
    );

    // The skipped commands settle under their IDs when sent again. The
    // archive commit and its acknowledgement advanced the economy runtime
    // revision, so the host rebuilds the economy command against the
    // current revision (the helper does): re-sent unchanged it would settle
    // as a stale rejection. Rebuilt, it is evaluated, here as a duplicate.
    let commands = [
        rejected_economy_command(&harness, "skipped"),
        rejected_force_command(&harness, "skipped"),
    ];
    settle_commands(&mut harness, 920, &commands);
    assert_eq!(
        economy(&harness).outcomes[&id("skipped")]
            .rejection_code
            .as_deref(),
        Some("duplicate_domain_record")
    );
    assert!(force(&harness).outcomes.contains_key(&force_id("skipped")));

    assert_restore_and_replay_agree(&mut harness);
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_result_past_the_operation_byte_share_is_skipped_and_maintenance_still_settles() {
    let mut harness = GrainHarness::with_limits(
        EconomyReferenceLimitsV1 {
            max_state_bytes: MIN_PACKAGE_STATE_BYTES,
            ..EconomyReferenceLimitsV1::canonical()
        },
        ForceSupplyReferenceLimitsV1::canonical(),
    )
    .expect("reference composition");
    let store = harness.archive_store();
    let share = package_operation_state_bytes(MIN_PACKAGE_STATE_BYTES);
    // Long operation IDs make each recorded rejection about 600 bytes.
    let long = |index: u64| format!("fill-{index:04}-{}", "x".repeat(140));
    let mut next = 0_u64;
    let mut skipped = None;
    while skipped.is_none() {
        assert!(next < 2_000, "the root never reached its operation share");
        let commands = (next..next + 150)
            .map(|index| rejected_economy_command(&harness, &long(index)))
            .collect::<Vec<_>>();
        let events = harness.canwu().events().len();
        settle_commands(&mut harness, 10_000 + next, &commands);
        next += 150;
        skipped = harness.canwu().events()[events..]
            .iter()
            .find_map(economy_operation_skip);
    }
    let skipped = skipped.expect("a skip");
    assert_eq!(skipped.shortage, PackageRootShortageV1::RootLimit);
    let state = economy(&harness);
    assert!(state.encoded_len().expect("length") <= share);
    let skipped_id = EconomyOperationId::new(skipped.operation_id.clone()).expect("ID");
    assert!(!state.outcomes.contains_key(&skipped_id));

    // Archive maintenance uses the reserve: a commit of one record applies
    // and two copies of it settle stale, each leaving a receipt beside the
    // handle, and the acknowledgement settles.
    let prepared = state
        .prepare_economy_archive(1, store.as_ref(), Some(&force(&harness)))
        .expect("the batch prepares");
    let mut queued = Vec::new();
    for _ in 0..3 {
        queued.push(
            enqueue_economy_archive(harness.canwu_mut(), &prepared, store.as_ref())
                .expect("the commit queues"),
        );
    }
    settle_now(&mut harness);
    let state = economy(&harness);
    assert_eq!(state.archive_maintenance_receipts.len(), 3);
    assert!(
        state.encoded_len().expect("length") > share,
        "the maintenance writes went into the reserve"
    );
    finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), &queued[0])
        .expect("the acknowledgement queues");
    settle_now(&mut harness);
    assert!(economy(&harness).archive_maintenance_receipts.is_empty());

    // An archive commit that clears the backlog makes room, and the
    // skipped command, rebuilt against the current revision, settles.
    let prepared = economy(&harness)
        .prepare_economy_archive(512, store.as_ref(), Some(&force(&harness)))
        .expect("the batch prepares");
    let receipt = enqueue_economy_archive(harness.canwu_mut(), &prepared, store.as_ref())
        .expect("the commit queues");
    settle_now(&mut harness);
    finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), &receipt)
        .expect("the acknowledgement queues");
    settle_now(&mut harness);
    let command = rejected_economy_command(
        &harness,
        skipped_id
            .as_str()
            .trim_start_matches("canwu.economy-reference:operation:"),
    );
    settle_commands(&mut harness, 9_000, &[command]);
    assert!(economy(&harness).outcomes.contains_key(&skipped_id));
}

#[test]
fn an_economy_commit_leaves_out_an_externality_outcome_the_force_root_holds_hot() {
    let mut harness = GrainHarness::new().expect("reference composition");
    for decision in [
        GrainDecision::Balanced,
        GrainDecision::ReliefFirst,
        GrainDecision::Balanced,
        GrainDecision::ForceFirst,
        GrainDecision::RequisitionForForce,
    ] {
        harness.advance_month(decision).expect("month");
    }
    let store = harness.archive_store();
    let force_state = force(&harness);
    let state = economy(&harness);
    let hot = state
        .externality_outcomes
        .keys()
        .find(|id| {
            force_state.terminal_receipts.values().any(|receipt| {
                receipt
                    .externality_outcome
                    .as_ref()
                    .map(|outcome| &outcome.id)
                    == Some(*id)
            })
        })
        .cloned()
        .expect("an externality outcome the force-supply root holds hot");
    let key = EconomyArchiveKeyV1::ExternalityOutcome(hot.clone());

    // Prepared without the force-supply root the batch selects it, and the
    // enqueue helper, which prepares with the run's force-supply root,
    // refuses that batch before anything is queued.
    let unaware = state
        .prepare_economy_archive(512, store.as_ref(), None)
        .expect("the batch prepares");
    assert!(unaware.selected.contains(&key));
    assert!(enqueue_economy_archive(harness.canwu_mut(), &unaware, store.as_ref()).is_err());

    // Prepared with it the outcome stays in the root, and the commit settles.
    let aware = state
        .prepare_economy_archive(512, store.as_ref(), Some(&force_state))
        .expect("the batch prepares");
    assert!(!aware.selected.contains(&key));
    let receipt = enqueue_economy_archive(harness.canwu_mut(), &aware, store.as_ref())
        .expect("the commit queues");
    settle_now(&mut harness);
    finalize_economy_archive_retention(harness.canwu_mut(), store.as_ref(), &receipt)
        .expect("the acknowledgement queues");
    settle_now(&mut harness);
    let state = economy(&harness);
    assert!(state.externality_outcomes.contains_key(&hot));
    assert_eq!(state.archive_head.revision, 1);

    // Once the force-supply archive took the terminal receipt, the next
    // economy commit archives the outcome.
    harness
        .archive_reference_history()
        .expect("archive both roots");
    let state = economy(&harness);
    assert!(!state.externality_outcomes.contains_key(&hot));
    let archived = load_package_archive_record::<EconomyArchiveKeyV1, EconomyArchivePayloadV1>(
        ECONOMY_ARCHIVE_DOMAIN,
        &state.archive_head,
        harness.canwu(),
        &key,
    )
    .expect("lookup")
    .expect("the archived outcome");
    assert!(matches!(
        archived.payload,
        EconomyArchivePayloadV1::ExternalityOutcome(outcome) if outcome.id == hot
    ));
}
