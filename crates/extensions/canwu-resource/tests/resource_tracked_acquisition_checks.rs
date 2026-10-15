//! A completion acquisition sent as a tracked command passes the checks the
//! completion ingress makes of the same operation.
//!
//! A holder's tracked `Acquire` settles at the instant it is admitted, so it
//! must be eligible at that instant, and the evidence its eligibility
//! envelope persists must be the current version of its record. Admission
//! refuses an acquisition that fails either; before this change both were
//! admitted and settled as live acquisitions, with an eligibility time other
//! than their boundary or evidence that was never current. Settlement checks
//! them again with the report reservation, which admission does not read: an
//! acquisition whose reports cannot be reserved is admitted, and its packet
//! settles nothing, records no outcome, and is named by a skip event.

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, ErrorCode, Issuer,
    KnowledgeHolderRef, PayloadSchema, PersonId, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "fixture.resource";
const EVIDENCE_NAMESPACE: &str = "fixture.provider";
const HOLDER: u64 = 1;

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(HOLDER))
}

/// The evidence an acquisition's envelope cites, at `version`.
fn evidence(version: u64) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, "evidence", "fixture"),
        version,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

struct EvidenceRecord;

impl DomainRecordType for EvidenceRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "evidence";
}

struct EvidencePlugin;

impl SimulationPlugin for EvidencePlugin {
    fn name(&self) -> &'static str {
        "fixture-provider"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f6"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<EvidenceRecord>();
        schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(schema)
    }
}

fn runtime() -> Canwu {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![CompletionCapacityPartitionV1 {
                    authority: holder(),
                    operation_namespace: NAMESPACE.to_owned(),
                    guaranteed_units: 400_000,
                    reserved_pending_slots: 8,
                    maximum_burst_units: 100_000,
                    request_token_capacity: 16,
                    request_token_refill_minutes: 1,
                    reacquire_cooldown_minutes: 1,
                    root_acquisition_cap_per_sim_time: 8,
                    guaranteed_max_wait_boundaries: 4,
                }],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
    let current = evidence(1);
    let record = DomainRecord {
        reference: current.record.clone(),
        owner: "fixture-provider".to_owned(),
        class: DomainRecordClass::Record,
        version: current.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "evidence": "fixture" }),
        references: Vec::new(),
    };
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(
            [vec![record], state.into_records().expect("resource record")].concat(),
        );
    let resource = ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    Canwu::new_with_plugins(5, scenario, &[&EvidencePlugin, &resource]).expect("runtime")
}

fn acquire(
    name: &str,
    eligible_at: SimTime,
    cited: DomainRecordVersionRef,
) -> ResourceOperationRequestV1 {
    acquire_reporting(name, eligible_at, cited, 0)
}

/// An acquisition that reserves `reports` reports for each of as many
/// holders.
fn acquire_reporting(
    name: &str,
    eligible_at: SimTime,
    cited: DomainRecordVersionRef,
    reports: u16,
) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Acquire(
        RequestCompletionLeaseV1 {
            id: CompletionLeaseAcquisitionId::new(format!("fixture:lease:{name}"))
                .expect("acquisition"),
            operation_key: ResourceOperationKey::new(format!("fixture:operation:{name}"))
                .expect("operation key"),
            holder: holder(),
            operation_namespace: NAMESPACE.to_owned(),
            eligibility_time: eligible_at,
            eligibility_envelope: EligibilityEnvelopeV1::new(
                vec![cited],
                BTreeMap::new(),
                BTreeSet::new(),
                Vec::new(),
                Vec::new(),
            )
            .expect("envelope"),
            recipe: CompletionCapacityRecipeV1 {
                receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                mutations: 2,
                reports_per_holder: reports,
                holders: reports,
                bytes: 1_024,
            },
            expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
            policy_class: CompletionPolicyClassV1::Guaranteed,
        },
    ))
}

/// Submits one tracked command and settles it when it is admitted. Returns
/// the admission outcome.
fn command(
    canwu: &mut Canwu,
    request_id: u64,
    request: ResourceOperationRequestV1,
) -> CommandAttemptOutcome {
    let at = canwu.time();
    let revision = canwu.revision();
    canwu
        .enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                revision,
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(HOLDER)),
                    resource_command(&ResourceCommandV1 {
                        subject: holder(),
                        request,
                    })
                    .expect("command"),
                )
                .at_time(at),
            ),
        )
        .expect("tracked command ingress");
    for _ in 0..2 {
        canwu
            .settle_boundary(BoundaryRequest::at(at))
            .expect("a resource boundary never fails in this fixture");
    }
    canwu
        .command_attempts()
        .last()
        .expect("command attempt")
        .outcome
        .clone()
}

fn acquisitions(canwu: &Canwu) -> Vec<String> {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
        .completion_leases
        .acquisitions
        .keys()
        .map(|id| id.as_str().to_owned())
        .collect()
}

#[test]
fn a_tracked_acquisition_is_refused_unless_eligible_now_on_current_evidence() {
    let mut canwu = runtime();
    let now = canwu.time();

    // Eligible a minute from now: refused when admitted.
    let early = command(
        &mut canwu,
        1,
        acquire("early", now + SimDuration::minutes(1), evidence(1)),
    );
    let CommandAttemptOutcome::Rejected { error } = early else {
        panic!("an acquisition eligible later is admitted: {early:?}");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);

    // Citing a version of the evidence that is not its record's current one.
    let stale = command(&mut canwu, 2, acquire("stale", now, evidence(2)));
    let CommandAttemptOutcome::Rejected { error } = stale else {
        panic!("an acquisition on evidence that is not current is admitted: {stale:?}");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert!(acquisitions(&canwu).is_empty());

    // Eligible now on the current version: admitted, and its packet passes
    // the same checks at settlement.
    let current = command(&mut canwu, 3, acquire("current", now, evidence(1)));
    assert!(
        matches!(current, CommandAttemptOutcome::Accepted { .. }),
        "{current:?}"
    );
    assert_eq!(acquisitions(&canwu), ["fixture:lease:current"]);
    assert!(
        !canwu
            .events()
            .iter()
            .any(|event| ResourceIngressSkipV1::from_event(event).is_some())
    );
}

#[test]
fn a_tracked_acquisition_whose_reports_cannot_be_reserved_settles_nothing() {
    let mut canwu = runtime();
    let now = canwu.time();
    // 200 reports for each of 200 holders pass the hot report capacity.
    // Admission does not read report capacity, so the command is admitted;
    // settlement checks it as the completion ingress does.
    let request = acquire_reporting("reporting", now, evidence(1), 200);
    let operation_key = request.operation_key();
    let events = canwu.events().len();
    let admitted = command(&mut canwu, 1, request);
    assert!(
        matches!(admitted, CommandAttemptOutcome::Accepted { .. }),
        "{admitted:?}"
    );
    let skipped: Vec<_> = canwu.events()[events..]
        .iter()
        .filter_map(ResourceIngressSkipV1::from_event)
        .collect();
    assert_eq!(skipped.len(), 1);
    assert_eq!(
        skipped[0].reason,
        ResourceIngressSkipReasonV1::FailedSettlementChecks
    );
    assert_eq!(skipped[0].operation_key, Some(operation_key.clone()));
    let root = resource_state(&canwu)
        .expect("resource query")
        .expect("resource state")
        .1;
    assert!(!root.outcomes.contains_key(&operation_key));
    assert!(root.completion_leases.acquisitions.is_empty());
}
