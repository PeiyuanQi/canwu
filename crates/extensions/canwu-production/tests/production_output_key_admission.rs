//! An execution's outputs name the operation keys under which the resource
//! plugin will credit them, and the key of the completion lease that backs
//! the execution is its first output's key. The resource plugin settles
//! nothing for a credit whose key it reserves for its completion operations
//! (`RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`) or whose key another request
//! already holds, so an execution started with such a key would never receive
//! its output acknowledgement: it would keep its capacity, its consumed
//! completion grants, and their target locks for good.
//!
//! `StartExecution` therefore refuses an output key that is reserved, that
//! another unsettled execution names, or that a resource outcome already
//! holds in the resource root or its archive, and a completion lease request
//! refuses a reserved key. Each is an expected domain rejection: the state API
//! returns the refusal and changes nothing, the plugin records it as the
//! operation's rejected outcome while its boundary settles, and a root that
//! holds such executions does not validate.
//!
//! The resource plugin settles every output in one batch, so `StartExecution`
//! also refuses an output set no batch can settle: more outputs than one
//! batch carries, or more than the resource root can hold under its limits
//! even beside an empty archive backlog.

mod support;

use canwu_api::{Canwu, ErrorCode, SimTime};
use canwu_production::*;
use canwu_resource::{
    CompletionGrantStateV1, MAX_PRODUCTION_OUTPUT_BATCH_CREDITS,
    RESOURCE_COMPLETION_OPERATION_KEY_PREFIX, ResourceCancelDemandRequestV1, ResourceDemandId,
    ResourceLimitsV1, ResourceOperationKey, ResourceOperationRequestV1, ResourcePlugin,
    ResourceRevision, ResourceState, ResourceTerminalRecordKeyV1, canonical_digest,
};
use std::sync::Arc;
use support::*;

fn reserved_key() -> ResourceOperationKey {
    ResourceOperationKey::new(format!(
        "{RESOURCE_COMPLETION_OPERATION_KEY_PREFIX}participant-complete:fixture"
    ))
    .expect("reserved key")
}

/// A root whose process has two outputs, so that an execution's second
/// output key is its own choice and not the lease's, and whose facility
/// serves two executions at once.
fn two_output_state() -> ProductionState {
    let mut state = base_state();
    let process = state
        .processes
        .get_mut(&process().id)
        .expect("household process");
    let mut second = process.outputs[0].clone();
    "coarse".clone_into(&mut second.quality_class);
    process.outputs.push(second);
    state
        .facilities
        .get_mut(&facility_id())
        .expect("fixture facility")
        .capacity
        .insert("bench".to_owned(), 2);
    state
}

/// `lease`'s start with its second output under `key`.
fn start_with_second_output(
    lease: &ActivatedLease,
    key: ResourceOperationKey,
) -> ProductionOperation {
    let mut start = start_operation(lease);
    let ProductionOperation::StartExecution { execution, .. } = &mut start else {
        unreachable!("the fixture builds a start");
    };
    execution.output_requests[1].operation_key = key;
    start
}

/// `lease`'s start with `count` outputs, each under its own key.
fn start_with_outputs(lease: &ActivatedLease, count: usize) -> ProductionOperation {
    let mut start = start_operation(lease);
    let ProductionOperation::StartExecution { execution, .. } = &mut start else {
        unreachable!("the fixture builds a start");
    };
    let template = execution.output_requests[1].clone();
    for index in execution.output_requests.len()..count {
        let mut output = template.clone();
        output.operation_key =
            ResourceOperationKey::new(format!("fixture:wide-output:{index}")).expect("key");
        execution.output_requests.push(output);
    }
    start
}

fn outcome_of<'a>(state: &'a ProductionState, id: &str) -> &'a ProductionOperationOutcome {
    &state.operation_outcomes[&ProductionOperationOutcomeId::new(id).expect("operation ID")]
}

fn ready_state(suffixes: &[&str]) -> (ProductionState, Vec<ActivatedLease>) {
    let mut state = two_output_state();
    let leases = suffixes
        .iter()
        .map(|suffix| activate_lease(&mut state, suffix))
        .collect();
    state.observation_dirty_index.clear();
    state.observation_due_index.clear();
    (state, leases)
}

#[test]
fn an_output_naming_a_reserved_key_is_refused_when_the_execution_starts() {
    let (mut state, leases) = ready_state(&["reserved"]);
    let lease = &leases[0];
    let ready = state.clone();
    let execution =
        ProductionExecutionId::new("production:execution:reserved").expect("execution ID");
    let reserved_start = start_with_second_output(lease, reserved_key());

    // The state API refuses the start and changes nothing.
    let error = state
        .apply_operation(
            &command(&state, "production:start:reserved", reserved_start.clone()),
            lease.at,
        )
        .expect_err("a reserved output key is refused");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    assert_eq!(state, ready);

    // Through the plugin, the refusal is the operation's recorded rejection
    // and its boundary settles. Nothing is consumed or started.
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut canwu = runtime(&plugin, ready.clone(), &archive);
    enqueue_operation(&mut canwu, 1, "production:start:reserved", reserved_start);
    settle(&mut canwu);
    settle(&mut canwu);
    let settled = production_state(&canwu);
    let outcome = outcome_of(&settled, "production:start:reserved");
    assert_eq!(
        outcome.disposition,
        ProductionOperationDisposition::Rejected
    );
    assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_payload"));
    assert!(settled.executions.is_empty());
    assert!(settled.capacity_allocations.is_empty());
    assert_eq!(
        settled.work_orders[&lease.order].lifecycle,
        WorkOrderLifecycle::Authorized
    );
    assert_eq!(
        settled.production_completion_grants[&lease.production_grant].state,
        CompletionGrantStateV1::Prepared
    );

    // The same start with its own output keys applies, so the refusal is the
    // key's alone.
    let mut started = ready;
    started
        .apply_operation(
            &command(
                &started,
                "production:start:ordinary",
                start_operation(lease),
            ),
            lease.at,
        )
        .expect("ordinary output keys start");
    started.validate().expect("the started root validates");

    // A root that holds an execution with a reserved output key, such as a
    // scenario assembled by hand, does not validate.
    let mut forged = started;
    forged
        .executions
        .get_mut(&execution)
        .expect("started execution")
        .output_requests[1]
        .operation_key = reserved_key();
    assert!(forged.validate().is_err());
}

#[test]
fn a_completion_lease_naming_a_reserved_key_is_refused() {
    // A first lease installs the run budget; the second request is valid but
    // for its key.
    let (mut state, _) = ready_state(&["first"]);
    let ready = state.clone();
    let at = SimTime::from_minutes(1);

    let error = state
        .request_completion_acquisition(lease_request("reserved", reserved_key(), at))
        .expect_err("a reserved lease key is refused");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    assert_eq!(state, ready);

    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut canwu = runtime(&plugin, ready, &archive);
    canwu
        .settle_boundary(canwu_api::BoundaryRequest::at(at))
        .expect("advance to the eligibility time");
    for (request_id, (operation_id, key)) in [
        ("production:lease:reserved", reserved_key()),
        (
            "production:lease:ordinary",
            ResourceOperationKey::new("resource:output-key:ordinary").expect("key"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        enqueue_operation(
            &mut canwu,
            u64::try_from(request_id + 1).expect("request ID"),
            operation_id,
            ProductionOperation::RequestCompletionLease {
                request: lease_request(operation_id.rsplit(':').next().expect("suffix"), key, at),
            },
        );
        settle(&mut canwu);
        settle(&mut canwu);
    }
    let settled = production_state(&canwu);
    let reserved = outcome_of(&settled, "production:lease:reserved");
    assert_eq!(
        reserved.disposition,
        ProductionOperationDisposition::Rejected
    );
    assert_eq!(reserved.rejection_code.as_deref(), Some("invalid_payload"));
    assert_eq!(
        outcome_of(&settled, "production:lease:ordinary").disposition,
        ProductionOperationDisposition::Applied
    );
    assert_eq!(settled.completion_acquisitions.len(), 2);
}

#[test]
fn an_output_key_another_unsettled_execution_names_is_refused() {
    let (mut state, leases) = ready_state(&["first", "second"]);
    let (first, second) = (&leases[0], &leases[1]);
    state
        .apply_operation(
            &command(&state, "production:start:first", start_operation(first)),
            first.at,
        )
        .expect("the first execution starts");
    let running = state.clone();
    let taken = first.outputs[1].operation_key.clone();

    let error = state
        .apply_operation(
            &command(
                &state,
                "production:start:second",
                start_with_second_output(second, taken.clone()),
            ),
            second.at,
        )
        .expect_err("a key the running execution names is refused");
    assert_eq!(error.code, ErrorCode::IdempotencyConflict);
    assert_eq!(state, running);

    // With its own keys the second execution starts beside the first, and a
    // root in which the two share an output key does not validate.
    state
        .apply_operation(
            &command(&state, "production:start:second", start_operation(second)),
            second.at,
        )
        .expect("the second execution starts with its own keys");
    state.validate().expect("two executions with distinct keys");
    let mut forged = state;
    forged
        .executions
        .get_mut(&ProductionExecutionId::new("production:execution:second").expect("ID"))
        .expect("second execution")
        .output_requests[1]
        .operation_key = taken;
    let error = forged.validate().expect_err("shared output key");
    assert!(error.message.contains("output operation key"), "{error}");
}

/// A resource root holding one hot outcome under `hot` and, in its archive,
/// one under `archived`: each a durable rejection of an operation under that
/// key. The archive commit is stored in `archive` and applied to the detached
/// root as its boundary would apply it.
fn resource_root_holding(
    hot: &ResourceOperationKey,
    archived: &ResourceOperationKey,
    archive: &TestArchive,
) -> ResourceState {
    let mut resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource");
    for key in [archived, hot] {
        resource
            .apply_operation(&ResourceOperationRequestV1::CancelDemand(
                ResourceCancelDemandRequestV1 {
                    operation_key: key.clone(),
                    demand: ResourceDemandId::new(format!("fixture:missing:{}", key.as_str()))
                        .expect("demand ID"),
                    expected_demand_revision: ResourceRevision::INITIAL,
                },
            ))
            .expect("durable rejection");
    }
    let batch = resource
        .prepare_resource_archive(1, archive)
        .expect("archive the first outcome");
    assert_eq!(
        batch.selected,
        [ResourceTerminalRecordKeyV1::Outcome(archived.clone())]
    );
    batch.store_and_verify(archive).expect("store the commit");
    resource.outcomes.remove(archived);
    resource
        .terminal_archive_candidates
        .retain(|_, key| key != &ResourceTerminalRecordKeyV1::Outcome(archived.clone()));
    resource
        .refresh_hot_terminal_ledger()
        .expect("the ledger counts the candidates placed directly");
    resource.archive_head.revision += 1;
    resource.archive_head.directory_root = Some(batch.directory.id.clone());
    resource.archive_head.archived_record_count += 1;
    resource.archive_head.semantic_digest.clear();
    resource.archive_head.semantic_digest =
        canonical_digest("canwu.resource.archive-head.v1", &resource.archive_head)
            .expect("archive head digest");
    resource
}

#[test]
fn an_output_key_a_resource_outcome_holds_is_refused_when_the_execution_starts() {
    let (state, leases) = ready_state(&["held"]);
    let lease = &leases[0];
    let archive = TestArchive::default();
    let hot = ResourceOperationKey::new("fixture:held:hot").expect("key");
    let archived = ResourceOperationKey::new("fixture:held:archived").expect("key");
    let resource = resource_root_holding(&hot, &archived, &archive);
    let mut scenario = scenario(state);
    scenario
        .domain_records
        .extend(resource.into_records().expect("resource root"));
    let production_plugin = ProductionPlugin;
    let resource_plugin = ResourcePlugin::default();
    let plugins: [&dyn canwu_api::SimulationPlugin; 2] = [&production_plugin, &resource_plugin];
    let mut canwu = Canwu::new_with_plugins(58, scenario, &plugins).expect("runtime");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));

    // A key a hot resource outcome holds, and one its archive holds, are each
    // the start's recorded rejection. A free key passes them and is then
    // judged on the start's evidence, which this fixture does not provide.
    for (request_id, (operation_id, key)) in [
        ("production:start:hot", hot),
        ("production:start:archived", archived),
        (
            "production:start:free",
            ResourceOperationKey::new("fixture:held:free").expect("key"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        enqueue_operation(
            &mut canwu,
            u64::try_from(request_id + 1).expect("request ID"),
            operation_id,
            start_with_second_output(lease, key),
        );
        settle(&mut canwu);
        settle(&mut canwu);
    }
    let settled = production_state(&canwu);
    for operation_id in ["production:start:hot", "production:start:archived"] {
        let outcome = outcome_of(&settled, operation_id);
        assert_eq!(
            outcome.disposition,
            ProductionOperationDisposition::Rejected
        );
        assert_eq!(
            outcome.rejection_code.as_deref(),
            Some("idempotency_conflict"),
            "{operation_id}: {:?}",
            outcome.rejection_message
        );
    }
    let free = outcome_of(&settled, "production:start:free");
    assert_eq!(free.disposition, ProductionOperationDisposition::Rejected);
    assert_ne!(free.rejection_code.as_deref(), Some("idempotency_conflict"));
    assert!(settled.executions.is_empty());
}

#[test]
fn an_output_set_no_resource_batch_can_settle_is_refused_when_the_execution_starts() {
    let (mut state, leases) = ready_state(&["wide"]);
    let lease = &leases[0];
    let ready = state.clone();
    let wide = start_with_outputs(lease, MAX_PRODUCTION_OUTPUT_BATCH_CREDITS + 1);

    // More outputs than one resource output batch carries: the state API
    // refuses the start and changes nothing.
    let error = state
        .apply_operation(
            &command(&state, "production:start:wide", wide.clone()),
            lease.at,
        )
        .expect_err("more outputs than one batch carries are refused");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    assert_eq!(state, ready);

    // A resource root whose archive candidate budget holds the execution's
    // two credits, but not beside the receipt of the archive commit that
    // empties its backlog: the batch would be skipped as never fitting.
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 2;
    let resource = ResourceState::empty(limits).expect("resource");
    assert!(resource.production_output_batch_can_fit(1));
    assert!(!resource.production_output_batch_can_fit(2));
    let mut scenario = scenario(ready);
    scenario
        .domain_records
        .extend(resource.into_records().expect("resource root"));
    let production_plugin = ProductionPlugin;
    let resource_plugin = ResourcePlugin::default();
    let plugins: [&dyn canwu_api::SimulationPlugin; 2] = [&production_plugin, &resource_plugin];
    let mut canwu = Canwu::new_with_plugins(59, scenario, &plugins).expect("runtime");
    canwu.set_plugin_archive_object_provider(Arc::new(TestArchive::default()));

    // Through the plugin each is the start's recorded rejection, and its
    // boundary settles.
    for (request_id, (operation_id, start)) in [
        ("production:start:wide", wide),
        ("production:start:unheld", start_operation(lease)),
    ]
    .into_iter()
    .enumerate()
    {
        enqueue_operation(
            &mut canwu,
            u64::try_from(request_id + 1).expect("request ID"),
            operation_id,
            start,
        );
        settle(&mut canwu);
        settle(&mut canwu);
    }
    let settled = production_state(&canwu);
    for (operation_id, message) in [
        (
            "production:start:wide",
            "execution output exceeds the credits one resource output batch carries",
        ),
        (
            "production:start:unheld",
            "execution output is a batch the resource root can never hold under its limits",
        ),
    ] {
        let outcome = outcome_of(&settled, operation_id);
        assert_eq!(
            outcome.disposition,
            ProductionOperationDisposition::Rejected
        );
        assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_payload"));
        assert_eq!(outcome.rejection_message.as_deref(), Some(message));
    }
    assert!(settled.executions.is_empty());
}
