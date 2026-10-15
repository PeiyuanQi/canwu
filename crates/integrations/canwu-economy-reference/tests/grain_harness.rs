use canwu_api::{
    BoundaryRequest, Command, CommandAuthority, CommandEnvelope, CommandRequest, CommandRequestId,
    DecisionAction, DecisionAttemptOutcome, DecisionContext, DecisionIngressRequest,
    DecisionMutation, DecisionRequestId, DecisionTicketDraft, DecisionTicketId,
    DecisionTicketState, EntityRef, Issuer, KnowledgeHolderRef, PersonId, PolicyDecision,
};
use canwu_api::{ItineraryRevisionReason, LegExecutionStatus};
use canwu_economy_reference::{
    ECONOMY_ARCHIVE_BLOB_NAMESPACE, GrainDecision, GrainHarness, economy_reference_state,
};
use canwu_force_supply_reference::{
    ExternalityOutcomeDisposition, ForceSupplyRuntimeRecord, force_supply_runtime_reference,
};
use canwu_resource::{
    RESOURCE_ARCHIVE_BLOB_NAMESPACE, ResourceArchiveRetentionHandleV1,
    ResourceArchiveRetentionPhaseV1, ResourceArchiveStore, ResourceError, ResourceOperationKey,
    ResourceOperationKind, ResourceOperationStatus, ResourceReportDtoV1,
    ResourceTerminalArchivePayloadV1, ResourceTerminalRecordKeyV1, ResourceTransferId,
    archived_resource_operation_outcome, archived_resource_terminal_record, resource_state,
};
use std::rc::Rc;

/// Reads the resource archive through the archive provider the runtime holds.
struct RuntimeArchive<'a>(&'a canwu_api::Canwu);

impl ResourceArchiveStore for RuntimeArchive<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the runtime archive view is read-only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        self.0
            .plugin_archive_object(namespace, object_id)
            .map_err(|error| ResourceError::InvalidDefinition(error.to_string()))
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the runtime archive view is read-only".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the runtime archive view is read-only".to_owned(),
        ))
    }
}

fn decisions() -> [GrainDecision; 14] {
    [
        GrainDecision::Balanced,
        GrainDecision::ReliefFirst,
        GrainDecision::Balanced,
        GrainDecision::ForceFirst,
        GrainDecision::RequisitionForForce,
        GrainDecision::Balanced,
        GrainDecision::ReliefFirst,
        GrainDecision::Balanced,
        GrainDecision::ForceFirst,
        GrainDecision::Balanced,
        GrainDecision::ReliefFirst,
        GrainDecision::Balanced,
        GrainDecision::ForceFirst,
        GrainDecision::Balanced,
    ]
}

#[test]
fn resolved_resilience_ticket_drives_the_same_months_allocation() {
    let mut base = GrainHarness::new().expect("real Canwu composition");
    for decision in decisions().into_iter().take(4) {
        base.advance_month(decision).expect("prefix month");
    }
    // Month 4 chose force first. Month 5 is scarce, so its own choice must
    // decide whether civilians and relief outrank the garrison dispatch.
    let balanced = base
        .fork()
        .expect("balanced fork")
        .advance_month(GrainDecision::Balanced)
        .expect("balanced month");
    let force_first = base
        .fork()
        .expect("force-first fork")
        .advance_month(GrainDecision::ForceFirst)
        .expect("force-first month");
    assert_eq!(balanced.civilian_fulfilled, balanced.civilian_requested);
    assert_eq!(balanced.relief_fulfilled, balanced.relief_requested);
    assert!(force_first.civilian_fulfilled < force_first.civilian_requested);
    assert_eq!(force_first.relief_fulfilled, 0);
}

#[test]
#[allow(clippy::too_many_lines)]
fn grain_ticket_rejects_a_command_from_a_different_option() {
    let mut harness = GrainHarness::new().expect("real Canwu composition");
    harness
        .advance_month(GrainDecision::Balanced)
        .expect("resolved source ticket");
    let source = harness
        .canwu()
        .decision_ticket(DecisionTicketId::new(1))
        .expect("source grain ticket")
        .clone();
    let controller = harness
        .canwu()
        .decision_controller(&source.assigned_controller)
        .expect("grain controller")
        .clone();
    let ticket_id = DecisionTicketId::new(900_000);
    let open_request = DecisionRequestId::new(900_001);
    let now = harness.canwu().time();
    let revision = harness.canwu().revision();
    harness
        .canwu_mut()
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                open_request,
                revision,
                DecisionMutation::Open {
                    ticket: DecisionTicketDraft {
                        id: ticket_id,
                        definition: source.definition,
                        decision_maker: EntityRef::Person(PersonId::new(1)),
                        assigned_controller: controller.id.clone(),
                        summary: "Mismatch rejection regression".to_owned(),
                        context: DecisionContext::new(
                            "canwu.economy-reference.grain-allocation.v1",
                            serde_json::json!({ "regression": "mismatched-option-command" }),
                        ),
                        options: source.options.clone(),
                        deadline: Some(now),
                        parent_ticket: None,
                    },
                },
            ),
        )
        .expect("open mismatch ticket");
    harness
        .canwu_mut()
        .settle_boundary(BoundaryRequest::at(now))
        .expect("open boundary");
    let ticket = harness
        .canwu()
        .decision_ticket(ticket_id)
        .expect("opened mismatch ticket");
    let selected = ticket.options[0].id.clone();
    let DecisionAction::Command { command } = &ticket.options[1].action else {
        panic!("grain option must contain a command");
    };
    let wrong_command: Command =
        serde_json::from_value(command.clone()).expect("other option command");
    let resolve_request = DecisionRequestId::new(900_002);
    let command_request = CommandRequestId::new(900_003);
    let revision = harness.canwu().revision();
    harness
        .canwu_mut()
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                resolve_request,
                revision,
                DecisionMutation::Resolve {
                    ticket_id,
                    expected_version: 1,
                    controller_id: controller.id.clone(),
                    policy: controller.policy.clone(),
                    decision: PolicyDecision::selected(
                        selected,
                        "intentionally paired with the wrong option command",
                    ),
                    command_request_id: Some(command_request),
                },
            )
            .with_command(CommandRequest::new(
                command_request,
                revision,
                CommandEnvelope::new(Issuer::Human(controller.id), wrong_command)
                    .with_authority(CommandAuthority::for_actor(PersonId::new(1)))
                    .at_time(now),
            )),
        )
        .expect("queue mismatched resolution");
    harness
        .canwu_mut()
        .settle_boundary(BoundaryRequest::at(now))
        .expect("mismatch is a stable rejected attempt");
    assert!(matches!(
        harness
            .canwu()
            .decision_attempt(resolve_request)
            .expect("mismatch attempt")
            .outcome,
        DecisionAttemptOutcome::Rejected { .. }
    ));
    assert!(matches!(
        harness
            .canwu()
            .decision_ticket(ticket_id)
            .expect("ticket remains unresolved")
            .state,
        DecisionTicketState::Open
    ));
}

#[test]
fn fourteen_month_composition_conserves_grain_and_persists_transport() {
    let harness = GrainHarness::new().expect("real Canwu composition");
    let summary = harness
        .run_fourteen_months(decisions())
        .expect("fourteen month run");
    assert_eq!(summary.frames.len(), 14);
    assert_eq!(summary.closed_route_months, vec![1, 2]);
    assert_eq!(summary.rerouted_months, vec![3]);
    assert_eq!(summary.transport_executions, 1);
    assert!(summary.total_harvest > 0);
    assert!(summary.conservation_closing > 0);
    assert!(summary.final_population_wellbeing_per_mille > 0);
    assert!(summary.frames.iter().any(|frame| {
        frame.civilian_fulfilled == frame.civilian_requested && frame.civilian_requested > 0
    }));
    assert!(summary.frames.iter().any(|frame| {
        frame.decision == GrainDecision::ReliefFirst
            && frame.relief_fulfilled > 0
            && frame.relief_fulfilled <= frame.relief_requested
    }));
    let requisition = &summary.frames[4];
    assert_eq!(requisition.decision, GrainDecision::RequisitionForForce);
    assert!(requisition.evidence.force_operation.is_some());
    assert!(requisition.civilian_fulfilled <= requisition.civilian_requested);
    assert!(requisition.relief_fulfilled <= requisition.relief_requested);
}

#[test]
#[allow(clippy::too_many_lines)]
fn canonical_resource_history_contains_real_leases_and_accepted_transport() {
    let mut harness = GrainHarness::new().expect("real Canwu composition");
    for decision in decisions().into_iter().take(3) {
        harness.advance_month(decision).expect("month");
    }
    let (_, state) = resource_state(harness.canwu())
        .expect("resource query")
        .expect("resource runtime");
    assert!(
        state
            .units
            .values()
            .all(|unit| unit.symbol == "synthetic-basket")
    );
    state.validate_conservation().expect("conservation");
    // A completed lease and a settled transfer leave the root with their
    // archived records, so what the root still holds follows the archive
    // cadence, not the lease or transfer contract. The lease of the month's
    // delivery and the delivery itself are read from wherever they now are.
    // The resource plugin is the only participant of the lease that starts
    // the delivery, so the loop takes it in a single step.
    let archive = RuntimeArchive(harness.canwu());
    let start_lease =
        ResourceOperationKey::new("resource:completion:lease:grain:lease:transfer-begin:03")
            .expect("single-step lease key");
    let leased = state
        .outcomes
        .get(&start_lease)
        .cloned()
        .or_else(|| {
            archived_resource_operation_outcome(&state, &archive, &start_lease)
                .expect("authenticated archive lookup")
        })
        .expect("the delivery's start lease is in the root or in its archive");
    assert_eq!(leased.kind, ResourceOperationKind::CompletionLease);
    assert_eq!(leased.status, ResourceOperationStatus::Applied);
    let delivery = ResourceTransferId::new("canwu.economy-reference:transfer:month-03")
        .expect("delivery transfer");
    let transfer = state
        .transfers
        .get(&delivery)
        .cloned()
        .or_else(|| {
            let archived = archived_resource_terminal_record(
                &state,
                &archive,
                &ResourceTerminalRecordKeyV1::Transfer(delivery.clone()),
            )
            .expect("authenticated archive lookup")?;
            match archived.payload {
                ResourceTerminalArchivePayloadV1::Transfer(transfer) => Some(transfer),
                _ => None,
            }
        })
        .expect("the delivery is in the root or in its archive");
    assert_eq!(
        transfer.state,
        canwu_resource::ResourceTransferState::Accepted
    );
    assert!(!transfer.exact_evidence.is_empty());
    assert!(state.outcomes.values().all(|outcome| {
        outcome.status == ResourceOperationStatus::Applied || outcome.rejection_reason.is_some()
    }));
    let manager_reports = harness
        .canwu()
        .knowledge()
        .for_holder(&KnowledgeHolderRef::Person(PersonId::new(1)))
        .expect("manager resource reports")
        .values()
        .filter_map(|record| {
            serde_json::from_value::<ResourceReportDtoV1>(record.payload.clone()).ok()
        })
        .collect::<Vec<_>>();
    assert!(
        manager_reports
            .iter()
            .any(|report| !report.demands.is_empty())
    );
    assert!(
        manager_reports
            .iter()
            .any(|report| !report.allocations.is_empty())
    );
    assert!(
        manager_reports
            .iter()
            .any(|report| !report.transfers.is_empty())
    );
    assert!(
        manager_reports
            .iter()
            .any(|report| !report.consumptions.is_empty())
    );
    let remote_reports = harness
        .canwu()
        .knowledge()
        .for_holder(&KnowledgeHolderRef::Person(PersonId::new(4)))
        .expect("delayed remote resource reports")
        .values()
        .filter_map(|record| {
            serde_json::from_value::<ResourceReportDtoV1>(record.payload.clone()).ok()
        })
        .collect::<Vec<_>>();
    assert!(remote_reports.iter().any(|report| report.stale));
    let force = harness
        .canwu()
        .typed_domain_record(&force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state");
    assert!(force.terminal_receipts.values().any(|receipt| {
        receipt
            .consequence
            .attribution
            .stock_custody
            .accepted_transfer
            .is_some()
    }));
    let (_, economy) = economy_reference_state(harness.canwu())
        .expect("economy query")
        .expect("economy runtime");
    let grain_ticket = harness
        .canwu()
        .decision_ticket(DecisionTicketId::new(1))
        .expect("G1b grain ticket");
    let DecisionTicketState::Resolved {
        option_id,
        trace_id,
    } = &grain_ticket.state
    else {
        panic!("G1b grain ticket must be resolved");
    };
    assert!(matches!(
        grain_ticket.option(option_id).map(|option| &option.action),
        Some(DecisionAction::Command { .. })
    ));
    assert!(
        harness
            .canwu()
            .decision_trace(*trace_id)
            .is_some_and(|trace| trace.command_request_id.is_some())
    );
    assert!(matches!(
        harness
            .canwu()
            .decision_attempt(DecisionRequestId::new(10_003))
            .expect("G1b resolved attempt")
            .outcome,
        DecisionAttemptOutcome::Accepted {
            command_request_id: Some(_),
            ..
        }
    ));
    let attempt = economy
        .delivery_attempts
        .values()
        .next()
        .expect("persisted delivery attempt");
    assert!(attempt.execution.revisions.iter().any(|revision| {
        matches!(
            &revision.reason,
            ItineraryRevisionReason::Disaster { explanation }
                if explanation == "river crossing closed; using ridge road"
        ) && revision.predecessor.is_some()
    }));
    assert!(attempt.execution.legs.iter().any(|leg| {
        leg.status == LegExecutionStatus::Failed
            && leg.failure_reason.as_deref() == Some("river crossing closed")
    }));
    for (ticket, resolve_request, schema, label) in [
        (
            DecisionTicketId::new(10_003),
            DecisionRequestId::new(20_007),
            "canwu.force-supply-reference.supply-choice.v1",
            "G4",
        ),
        (
            DecisionTicketId::new(20_003),
            DecisionRequestId::new(30_007),
            "canwu.economy-reference.local-resilience.v1",
            "G5",
        ),
    ] {
        let ticket = harness
            .canwu()
            .decision_ticket(ticket)
            .unwrap_or_else(|| panic!("persisted {label} ticket"));
        assert_eq!(ticket.context.schema, schema);
        let DecisionTicketState::Resolved { trace_id, .. } = ticket.state else {
            panic!("G4/G5 decision ticket must be resolved");
        };
        assert!(harness.canwu().decision_trace(trace_id).is_some());
        assert!(matches!(
            harness
                .canwu()
                .decision_attempt(resolve_request)
                .expect("persisted G4/G5 attempt")
                .outcome,
            DecisionAttemptOutcome::Accepted {
                trace_id: Some(_),
                ..
            }
        ));
    }
}

#[test]
fn snapshot_checkpoint_journal_and_fork_continue_identically() {
    let mut original = GrainHarness::new().expect("real Canwu composition");
    let plan = decisions();
    for decision in plan.into_iter().take(4) {
        original.advance_month(decision).expect("prefix month");
    }
    let snapshot = original.snapshot_json().expect("snapshot");
    let journal = original
        .checkpoint_journal_json()
        .expect("checkpoint journal");
    let mut loaded = GrainHarness::from_snapshot_json(&snapshot).expect("snapshot load");
    let mut replayed = GrainHarness::replay_from_journal_json(&journal).expect("journal replay");
    let mut forked = original.fork().expect("fork");
    for decision in decisions().into_iter().skip(4).take(2) {
        original
            .advance_month(decision)
            .expect("original continuation");
        loaded.advance_month(decision).expect("loaded continuation");
        replayed
            .advance_month(decision)
            .expect("replayed continuation");
        forked.advance_month(decision).expect("forked continuation");
    }
    for harness in [&original, &loaded, &replayed, &forked] {
        let ticket = harness
            .canwu()
            .decision_ticket(DecisionTicketId::new(6))
            .expect("continued grain ticket");
        let DecisionTicketState::Resolved {
            option_id,
            trace_id,
        } = &ticket.state
        else {
            panic!("continued grain ticket must remain resolved");
        };
        let option = ticket.option(option_id).expect("resolved option");
        assert_eq!(option.metadata["grain_decision"], "balanced");
        assert!(matches!(option.action, DecisionAction::Command { .. }));
        assert!(
            harness
                .canwu()
                .decision_trace(*trace_id)
                .is_some_and(|trace| trace.command_request_id.is_some())
        );
    }
    let expected = original
        .summary()
        .expect("original summary")
        .checkpoint_hash;
    assert_eq!(
        loaded.summary().expect("loaded summary").checkpoint_hash,
        expected
    );
    assert_eq!(
        replayed
            .summary()
            .expect("replayed summary")
            .checkpoint_hash,
        expected
    );
    assert_eq!(
        forked.summary().expect("forked summary").checkpoint_hash,
        expected
    );
}

#[test]
fn snapshot_restore_authenticates_the_embedded_resource_archive_batch() {
    let mut harness = GrainHarness::new().expect("real Canwu composition");
    for decision in decisions().into_iter().take(4) {
        harness
            .advance_month(decision)
            .expect("archive-producing month");
    }
    assert!(harness.archive_object_count() > 0);
    let snapshot = harness.snapshot_json().expect("snapshot");
    GrainHarness::from_snapshot_json(&snapshot).expect("verified archive restore");

    let mut forged: serde_json::Value = serde_json::from_str(&snapshot).expect("snapshot json");
    let objects = forged["archive"]["objects"]
        .as_array_mut()
        .expect("archive object array");
    let blob = objects
        .iter_mut()
        .find(|object| object["namespace"] == RESOURCE_ARCHIVE_BLOB_NAMESPACE)
        .expect("resource archive blob");
    blob["bytes"] = serde_json::json!([123, 125]);
    let forged = serde_json::to_string(&forged).expect("forged snapshot json");
    assert!(GrainHarness::from_snapshot_json(&forged).is_err());
}

#[test]
fn force_and_economy_archives_are_provider_backed_and_restore_authenticated() {
    let mut harness = GrainHarness::new().expect("real Canwu composition");
    for decision in decisions().into_iter().take(4) {
        harness.advance_month(decision).expect("archive prefix");
    }
    let force_before = harness
        .canwu()
        .typed_domain_record(&force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state");
    let retired_acquisitions = force_before
        .terminal_receipts
        .values()
        .map(|receipt| receipt.completion_certificate.acquisition.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!retired_acquisitions.is_empty());
    assert!(!force_before.outcomes.is_empty());
    let (_, economy_before) = economy_reference_state(harness.canwu())
        .expect("economy state")
        .expect("economy runtime");
    let frame_count_before = economy_before.frames.values().map(Vec::len).sum::<usize>();
    let observation_count_before = economy_before
        .observation_heads
        .values()
        .map(Vec::len)
        .sum::<usize>();
    assert!(frame_count_before > 0);
    assert!(observation_count_before > economy_before.observation_heads.len());
    assert!(!economy_before.outcomes.is_empty());
    harness
        .archive_reference_history()
        .expect("archive force/economy history");
    let force_after = harness
        .canwu()
        .typed_domain_record(&force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state");
    assert!(force_after.terminal_receipts.is_empty());
    assert!(force_after.outcomes.len() < force_before.outcomes.len());
    for acquisition in &retired_acquisitions {
        assert!(
            !force_after
                .completion_leases
                .acquisitions
                .contains_key(acquisition)
        );
        assert!(
            !force_after
                .completion_leases
                .certificates
                .contains_key(acquisition)
        );
        assert!(
            !force_after
                .completion_participant_grants
                .contains_key(acquisition)
        );
    }
    let (_, economy_after) = economy_reference_state(harness.canwu())
        .expect("economy state")
        .expect("economy runtime");
    assert!(economy_after.frames.values().map(Vec::len).sum::<usize>() < frame_count_before);
    assert!(
        economy_after
            .observation_heads
            .values()
            .map(Vec::len)
            .sum::<usize>()
            < observation_count_before
    );
    assert!(economy_after.outcomes.len() < economy_before.outcomes.len());
    let snapshot = harness.snapshot_json().expect("snapshot");
    GrainHarness::from_snapshot_json(&snapshot).expect("authenticated package archive restore");

    let mut forged_retention: serde_json::Value =
        serde_json::from_str(&snapshot).expect("snapshot json");
    let retention = forged_retention["archive"]["package_retention"]
        .as_object_mut()
        .expect("package retention map")
        .values_mut()
        .next()
        .expect("terminal package retention handle");
    retention["phase"] = serde_json::json!("verified");
    let forged_retention =
        serde_json::to_string(&forged_retention).expect("forged retention snapshot json");
    assert!(GrainHarness::from_snapshot_json(&forged_retention).is_err());

    let mut forged: serde_json::Value = serde_json::from_str(&snapshot).expect("snapshot json");
    let objects = forged["archive"]["objects"]
        .as_array_mut()
        .expect("archive object array");
    let blob = objects
        .iter_mut()
        .find(|object| {
            object["namespace"] == ECONOMY_ARCHIVE_BLOB_NAMESPACE
                || object["namespace"] == canwu_force_supply_reference::FORCE_ARCHIVE_BLOB_NAMESPACE
        })
        .expect("package archive blob");
    blob["bytes"] = serde_json::json!([123, 125]);
    let forged = serde_json::to_string(&forged).expect("forged snapshot json");
    assert!(GrainHarness::from_snapshot_json(&forged).is_err());
}

#[test]
fn requisition_branch_is_reproducible_and_carries_future_cost() {
    let mut base = GrainHarness::new().expect("real Canwu composition");
    for decision in decisions().into_iter().take(4) {
        base.advance_month(decision).expect("prefix month");
    }
    let mut balanced = base.fork().expect("balanced fork");
    let mut requisition_a = base.fork().expect("requisition fork A");
    let mut requisition_b = base.fork().expect("requisition fork B");
    balanced
        .advance_month(GrainDecision::Balanced)
        .expect("balanced decision");
    requisition_a
        .advance_month(GrainDecision::RequisitionForForce)
        .expect("requisition decision A");
    requisition_b
        .advance_month(GrainDecision::RequisitionForForce)
        .expect("requisition decision B");
    // Month 5 settles the saga with an applied externality, which leaves the
    // compiled penalty pending for the next harvest.
    let force = requisition_a
        .canwu()
        .typed_domain_record(&force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state");
    assert!(force.terminal_receipts.values().any(|receipt| {
        receipt.saga.is_some()
            && receipt.externality_outcome.as_ref().is_some_and(|outcome| {
                outcome.disposition == ExternalityOutcomeDisposition::Applied
            })
    }));
    let policy = force
        .requisition_policies
        .values()
        .next()
        .expect("compiled requisition policy");
    let cooperation_cost = policy.cooperation_delta_per_mille.unsigned_abs();
    let (_, economy) = economy_reference_state(requisition_a.canwu())
        .expect("economy query")
        .expect("economy runtime");
    let local = economy.local_economies.values().next().expect("economy");
    assert_eq!(
        local.pending_harvest_penalty_per_mille,
        policy.harvest_input_delta_per_mille.unsigned_abs()
    );
    for decision in decisions().into_iter().skip(5).take(5) {
        balanced.advance_month(decision).expect("balanced future");
        requisition_a
            .advance_month(decision)
            .expect("requisition future A");
        requisition_b
            .advance_month(decision)
            .expect("requisition future B");
    }
    // The month 10 harvest used the penalty up.
    let (_, economy) = economy_reference_state(requisition_a.canwu())
        .expect("economy query")
        .expect("economy runtime");
    let local = economy.local_economies.values().next().expect("economy");
    assert_eq!(local.pending_harvest_penalty_per_mille, 0);
    let balanced = balanced.summary().expect("balanced summary");
    let requisition_a = requisition_a.summary().expect("requisition summary A");
    let requisition_b = requisition_b.summary().expect("requisition summary B");
    assert_eq!(requisition_a.checkpoint_hash, requisition_b.checkpoint_hash);
    assert_ne!(balanced.checkpoint_hash, requisition_a.checkpoint_hash);
    // Month 5 applies the compiled cooperation cost exactly once.
    assert_eq!(
        requisition_a.frames[4].cooperation_per_mille + cooperation_cost,
        requisition_a.frames[3].cooperation_per_mille
    );
    // The month 10 harvest reads cooperation after month 9. The pending
    // penalty lowers the yield per cooperation point below the balanced one.
    let balanced_cooperation = u64::from(balanced.frames[8].cooperation_per_mille);
    let requisition_cooperation = u64::from(requisition_a.frames[8].cooperation_per_mille);
    assert!(
        requisition_a.total_harvest * balanced_cooperation
            < balanced.total_harvest * requisition_cooperation
    );
}

#[test]
fn requisition_externality_rejects_a_local_economy_changed_after_its_lock() {
    let mut harness = GrainHarness::new().expect("real Canwu composition");
    let mut prefix = None;
    for decision in decisions().into_iter().take(4) {
        prefix = Some(harness.advance_month(decision).expect("prefix month"));
    }
    let month_4 = prefix.expect("month 4 frame");
    // Another resolved grain ticket changes the local economy after the
    // requisition lease locked it, so the externality must not apply.
    harness.reselect_decision_during_requisition(true);
    let month_5 = harness
        .advance_month(GrainDecision::RequisitionForForce)
        .expect("requisition month");
    let force = harness
        .canwu()
        .typed_domain_record(&force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state");
    assert!(force.terminal_receipts.values().any(|receipt| {
        receipt.saga.is_some()
            && receipt.externality_outcome.as_ref().is_some_and(|outcome| {
                outcome.disposition == ExternalityOutcomeDisposition::Rejected
            })
    }));
    let (_, economy) = economy_reference_state(harness.canwu())
        .expect("economy query")
        .expect("economy runtime");
    let local = economy.local_economies.values().next().expect("economy");
    assert_eq!(local.pending_harvest_penalty_per_mille, 0);
    assert_eq!(month_5.cooperation_per_mille, month_4.cooperation_per_mille);
}

/// Serves the archive objects the provider attached to a runtime serves.
struct ArchiveObjectsOf(canwu_api::Canwu);

impl canwu_api::PluginArchiveObjectProvider for ArchiveObjectsOf {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_api::CanwuError> {
        self.0.plugin_archive_object(namespace, object_id)
    }
}

/// A force runtime cites resource outcomes a resource archive commit may
/// have archived. The force-supply restore attaches the archive before it
/// validates and reads an archived citation from it: it restores with the
/// archive and refuses without it. (The force-supply unit tests compare a
/// forged citation; a snapshot that carries one is refused by its boundary
/// hash chain before any plugin validates it.)
#[test]
fn a_force_runtime_citing_archived_resource_outcomes_restores_with_its_archive_only() {
    let mut harness = GrainHarness::new().expect("real Canwu composition");
    for decision in decisions().into_iter().take(4) {
        harness
            .advance_month(decision)
            .expect("archive-producing month");
    }
    let canwu = harness.canwu();
    let force = canwu
        .typed_domain_record(&force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force runtime payload");
    let (_, resource) = resource_state(canwu)
        .expect("resource root")
        .expect("resource runtime");
    assert!(
        force.terminal_receipts.values().any(|receipt| {
            let key = &receipt.resource_outcome.operation_key;
            !resource.outcomes.contains_key(key)
                && archived_resource_operation_outcome(&resource, &RuntimeArchive(canwu), key)
                    .expect("archive lookup")
                    .is_some()
        }),
        "a terminal receipt cites an archived resource outcome"
    );

    let economy_kind = canwu_economy_reference::economy_reference_runtime_reference()
        .into_untyped()
        .kind;
    let force_kind = force_supply_runtime_reference().into_untyped().kind;
    let resource = canwu_resource::ResourcePlugin::new([economy_kind, force_kind]);
    let economy = canwu_economy_reference::EconomyReferencePlugin;
    let force = canwu_force_supply_reference::ForceSupplyReferencePlugin;
    let plugins: [&dyn canwu_api::SimulationPlugin; 3] = [&resource, &economy, &force];
    let snapshot = canwu.snapshot_json().expect("snapshot");
    canwu_force_supply_reference::from_force_supply_snapshot_json(
        &snapshot,
        &plugins,
        Rc::new(ArchiveObjectsOf(canwu.fork())),
    )
    .expect("the force runtime restores with its archive");
    let Err(error) = canwu_force_supply_reference::from_force_supply_snapshot_json(
        &snapshot,
        &plugins,
        Rc::new(()),
    ) else {
        panic!("an archived citation is not taken on trust");
    };
    assert!(
        error.message.contains("resource archive lookup failed"),
        "{error:?}"
    );
    let checkpoint = canwu.checkpoint_journal().expect("checkpoint journal");
    canwu_force_supply_reference::from_force_supply_checkpoint_journal(
        checkpoint.clone(),
        &plugins,
        Rc::new(ArchiveObjectsOf(canwu.fork())),
    )
    .expect("the checkpoint restores with its archive");
    assert!(
        canwu_force_supply_reference::from_force_supply_checkpoint_journal(
            checkpoint,
            &plugins,
            Rc::new(()),
        )
        .is_err()
    );

    GrainHarness::from_snapshot_json(&harness.snapshot_json().expect("harness snapshot"))
        .expect("the harness restores and validates the archived outcomes");
}
