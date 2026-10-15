//! Downstream gap G-56: a custodian installs a protected-floor policy
//! revision at runtime and points its account at it.
//!
//! The first fixture is a plugin-owned run. The custodian names a floor of
//! any quantity through one tracked command, which leaves the outcome of any
//! token-only operation. A grantee of the custodian's stock cannot, a resend
//! is not a second change, and every refusal is recorded: on the command when
//! authority or the operation key refuses it, as the operation's rejected
//! outcome otherwise. A command admitted before the request that takes its
//! key has settled is left unsettled without failing its boundary. The floor
//! then changes every day for a month and the
//! root never holds more than the revisions something still refers to: a
//! revision is retired when no account and no demand in the root names it,
//! in the boundary of the archive commit that removes the demand when an
//! install is queued beside that commit, and sending its body installs it
//! again.
//!
//! The second fixture pins the bound on revisions in one root. The third runs
//! one script against three roots, with a reservation, an open demand, and a
//! granted demand already in flight when the floor changes. Two hold the same
//! content-addressed revision, one from its scenario and one from the
//! custodian's request, and are equal before and after: the root does not
//! record how a revision arrived. The third points at an authored revision of
//! the same content by id, and every operation has the same effect on it.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordRef,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, EntityRef, ErrorCode, EvidenceRef, Issuer, KnowledgeHolderRef,
    PayloadSchema, PersonId, PluginArchiveObjectProvider, PluginRegistrar, Scenario, SimDuration,
    SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

const NAMESPACE: &str = "fixture.resource";
const CUSTODIAN: u64 = 1;
const GRANTEE: u64 = 2;

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn minute(value: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::minutes(value)
}

fn day(value: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(value)
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn account(name: &str) -> ResourceAccountId {
    ResourceAccountId::new(format!("fixture:account:{name}")).expect("account")
}

fn demand_id(name: &str) -> ResourceDemandId {
    ResourceDemandId::new(format!("fixture:demand:{name}")).expect("demand")
}

fn grant_id() -> ResourceAccessGrantId {
    ResourceAccessGrantId::new("fixture:grant:stock").expect("grant")
}

fn evidence(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new("fixture.provider", "evidence", id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

/// The content-addressed revision for one floor quantity.
fn floor(quantity: u64) -> ProtectedFloorPolicyRevision {
    ProtectedFloorPolicyRevision::content_addressed(
        ResourceRevision::INITIAL,
        quantity,
        BTreeSet::new(),
    )
    .expect("content-addressed floor")
}

/// A revision as a scenario authors it: an identity and a digest of its
/// author's choosing.
fn authored(name: &str, quantity: u64) -> ProtectedFloorPolicyRevision {
    ProtectedFloorPolicyRevision {
        id: ProtectedFloorPolicyRevisionId::new(format!("fixture:floor:{name}")).expect("floor"),
        revision: ResourceRevision::INITIAL,
        floor: quantity,
        override_classes: BTreeSet::new(),
        semantic_digest: "a".repeat(64),
    }
}

fn partition(authority: KnowledgeHolderRef) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        authority,
        operation_namespace: NAMESPACE.to_owned(),
        guaranteed_units: 400_000,
        reserved_pending_slots: 8,
        maximum_burst_units: 100_000,
        request_token_capacity: 16,
        request_token_refill_minutes: 1,
        reacquire_cooldown_minutes: 1,
        root_acquisition_cap_per_sim_time: 4,
        guaranteed_max_wait_boundaries: 4,
    }
}

/// A root in which the custodian holds a granary of 100 units and an empty
/// annex, both without a floor, and has granted the grantee access to 40.
fn world() -> ResourceState {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![partition(holder(CUSTODIAN)), partition(holder(GRANTEE))],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
    let unit = ResourceUnitRevisionId::new("fixture:unit:measure:v1").expect("unit");
    state
        .install_unit(ResourceUnitRevision {
            id: unit.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "u".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .expect("unit");
    let resource = ResourceDefinitionRevisionId::new("fixture:resource:stock:v1").expect("id");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("fixture:resource:stock").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit.clone(),
            quality: ResourceQualityId::new("fixture:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("fixture:scope:store").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    for (name, balance) in [("granary", 100), ("annex", 0)] {
        state
            .install_opening_account(ResourceAccount {
                id: account(name),
                revision: ResourceRevision::INITIAL,
                custodian: holder(CUSTODIAN),
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                balance,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: None,
            })
            .expect("account");
    }
    applied(
        &mut state,
        ResourceOperationRequestV1::IssueAccessGrant(ResourceIssueAccessGrantRequestV1 {
            operation_key: key("fixture:grant:issue"),
            grant: ResourceAccessGrantV1 {
                grant_id: grant_id(),
                grantor_custodian: holder(CUSTODIAN),
                grantee: holder(GRANTEE),
                resource_revision: resource,
                unit_revision: unit,
                cap_quantity: 40,
                valid_from: SimTime::EPOCH,
                valid_until: day(400),
                authority_evidence: evidence("consent"),
            },
        }),
    );
    state
}

fn applied(
    state: &mut ResourceState,
    request: ResourceOperationRequestV1,
) -> ResourceOperationOutcome {
    let outcome = state.apply_operation(&request).expect("operation");
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
    outcome
}

fn rejected(
    state: &mut ResourceState,
    request: ResourceOperationRequestV1,
    code: &str,
) -> ResourceOperationOutcome {
    let outcome = state
        .apply_operation(&request)
        .expect("a refused operation is a recorded rejection");
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{code}");
    assert_eq!(
        outcome.rejection_code.as_deref(),
        Some(code),
        "{:?}",
        outcome.rejection_reason
    );
    outcome
}

/// A request that points `account` at `policy`, sending its body when
/// `install` is set.
fn set_floor(
    operation: &str,
    state: &ResourceState,
    name: &str,
    policy: Option<&ProtectedFloorPolicyRevision>,
    install: bool,
) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::SetProtectedFloor(ResourceProtectedFloorRequestV1 {
        operation_key: key(operation),
        account: account(name),
        expected_account_revision: state.accounts[&account(name)].revision,
        policy: policy.map(|policy| policy.id.clone()),
        install: policy.filter(|_| install).cloned(),
    })
}

/// A demand on the granary. The requester draws on its own account, or as
/// the grantee under the access grant.
fn demand(
    state: &ResourceState,
    name: &str,
    requester: u64,
    requested: u64,
    due_at: SimTime,
    policy: Option<&ProtectedFloorPolicyRevision>,
) -> ResourceDemand {
    let granary = &state.accounts[&account("granary")];
    ResourceDemand {
        id: demand_id(name),
        revision: ResourceRevision::INITIAL,
        requester: holder(requester),
        source_policy: if requester == GRANTEE {
            ResourceDemandSourcePolicyV1::Granted {
                grant_id: grant_id(),
                accounts: vec![account("granary")],
            }
        } else {
            ResourceDemandSourcePolicyV1::ExactAccounts(vec![account("granary")])
        },
        resource_revision: granary.resource_revision.clone(),
        unit_revision: granary.unit_revision.clone(),
        requested,
        fulfilled: 0,
        minimum_useful: requested,
        partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
        alternative_group: None,
        due_at,
        expires_at: due_at + SimDuration::days(300),
        priority: 1,
        tie_break: ResourceTieBreakKey::new(format!("fixture:tie:{name}")).expect("tie"),
        admitted_sequence: 0,
        protected_floor_policy: policy.map(|policy| policy.id.clone()),
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    }
}

fn submit(demand: ResourceDemand) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
        operation_key: key(&format!(
            "fixture:submit:{}",
            demand.id.as_str().replace(':', "-")
        )),
        demand,
    })
}

fn allocate(operation: &str, state: &ResourceState, at: SimTime) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::Allocate(ResourceAllocationRequestV1 {
        operation_key: key(operation),
        expected_state_revision: state.state_revision,
        at,
        candidate_limit: 16,
    })
}

fn protected(state: &ResourceState, name: &str) -> (u64, u64) {
    let quantities = state
        .account_quantities(&account(name))
        .expect("quantities");
    (quantities.protected, quantities.available)
}

fn floor_ids(state: &ResourceState) -> BTreeSet<ProtectedFloorPolicyRevisionId> {
    state.protected_floor_policies.keys().cloned().collect()
}

/// Owner of the evidence record the access grant cites.
struct EvidenceRecord;

impl DomainRecordType for EvidenceRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "fixture.provider";
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

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

#[derive(Clone, Default)]
struct MemoryArchive {
    objects: Rc<RefCell<ArchiveObjects>>,
    retention: Rc<RefCell<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
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
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .borrow_mut()
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

fn settle(canwu: &mut Canwu, at: SimTime) {
    canwu
        .settle_boundary(BoundaryRequest::at(at))
        .expect("a resource boundary never fails in this fixture");
}

/// A plugin-owned run and the next tracked command request id.
struct Run {
    canwu: Canwu,
    archive: MemoryArchive,
    commands: u64,
}

impl Run {
    /// Queues one tracked command from `actor` about itself. `ahead` counts
    /// the commands queued before it that the same boundary will admit: each
    /// admission advances the revision this command must expect.
    fn enqueue(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
        ahead: u64,
    ) {
        self.commands += 1;
        self.canwu
            .enqueue_command(
                at,
                0,
                CommandRequest::new(
                    CommandRequestId::new(self.commands),
                    self.canwu.revision() + ahead,
                    CommandEnvelope::new(
                        Issuer::Actor(PersonId::new(actor)),
                        resource_command(&ResourceCommandV1 {
                            subject: holder(actor),
                            request,
                        })
                        .expect("command"),
                    )
                    .at_time(at),
                ),
            )
            .expect("tracked command ingress");
    }

    /// Submits one tracked command from `actor` about itself and settles
    /// the boundary that admits it. An admitted command is then queued for
    /// the next boundary. Returns the admission outcome.
    fn admit(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
    ) -> CommandAttemptOutcome {
        self.enqueue(actor, at, request, 0);
        settle(&mut self.canwu, at);
        self.canwu
            .command_attempts()
            .last()
            .expect("command attempt")
            .outcome
            .clone()
    }

    /// Submits one tracked command from `actor` about itself and, when
    /// admitted, settles it. Returns the admission outcome.
    fn command(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
    ) -> CommandAttemptOutcome {
        let outcome = self.admit(actor, at, request);
        if matches!(outcome, CommandAttemptOutcome::Accepted { .. }) {
            settle(&mut self.canwu, at);
        }
        outcome
    }

    /// Settles one admitted command and returns its recorded outcome.
    fn settled(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
    ) -> ResourceOperationOutcome {
        let operation_key = request.operation_key();
        let outcome = self.command(actor, at, request);
        assert!(
            matches!(outcome, CommandAttemptOutcome::Accepted { .. }),
            "{outcome:?}"
        );
        state(&self.canwu).outcomes[&operation_key].clone()
    }

    /// A command refused at admission: recorded on the command, with the
    /// resource root untouched.
    fn refused(
        &mut self,
        actor: u64,
        at: SimTime,
        request: ResourceOperationRequestV1,
        code: ErrorCode,
    ) {
        let before = state(&self.canwu);
        let CommandAttemptOutcome::Rejected { error } = self.command(actor, at, request) else {
            panic!("the command was admitted");
        };
        assert_eq!(error.code, code, "{}", error.message);
        assert_eq!(state(&self.canwu), before);
    }

    /// Archives every hot terminal candidate in one commit and acknowledges it.
    fn archive_all(&mut self) {
        let hot = state(&self.canwu);
        let prepared = hot
            .prepare_resource_archive(hot.terminal_archive_candidates.len(), &self.archive)
            .expect("prepare archive commit");
        let receipt = enqueue_resource_archive(&mut self.canwu, &prepared, &self.archive)
            .expect("enqueue commit");
        let at = self.canwu.time();
        settle(&mut self.canwu, at);
        finalize_resource_archive_retention(&mut self.canwu, &self.archive, &receipt)
            .expect("finalize retention");
        settle(&mut self.canwu, at);
    }
}

#[test]
fn a_custodian_names_any_floor_and_the_root_stays_bounded() {
    let rung = authored("rung", 50);
    let mut resource = world();
    resource
        .install_protected_floor_policy(rung.clone())
        .expect("authored rung");
    let resource_plugin = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let consent = evidence("consent");
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(CUSTODIAN)),
            EntityRef::Person(PersonId::new(GRANTEE)),
        ],
    )
    .with_domain_records(
        [
            vec![DomainRecord {
                reference: consent.record.clone(),
                owner: "fixture-provider".to_owned(),
                class: DomainRecordClass::Record,
                version: consent.version,
                lifecycle: DomainRecordLifecycle::Active,
                payload: serde_json::json!({ "evidence": "consent" }),
                references: Vec::new(),
            }],
            resource.into_records().expect("resource record"),
        ]
        .concat(),
    );
    let archive = MemoryArchive::default();
    let mut canwu = Canwu::new_with_plugins(56, scenario, &plugins).expect("runtime");
    canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));
    let mut run = Run {
        canwu,
        archive,
        commands: 0,
    };
    let at = SimTime::EPOCH;

    // The custodian names a quantity no scenario authored. One command
    // installs the revision and points the account at it.
    let first_floor = floor(37);
    let first = set_floor(
        "fixture:floor:first",
        &state(&run.canwu),
        "granary",
        Some(&first_floor),
        true,
    );
    let outcome = run.settled(CUSTODIAN, at, first.clone());
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(outcome.kind, ResourceOperationKind::SetProtectedFloor);
    assert_eq!(outcome.quantity, 37);
    assert_eq!(outcome.result_ref, None);
    assert!(outcome.exact_evidence.is_empty());
    let current = state(&run.canwu);
    assert_eq!(
        current.accounts[&account("granary")].protected_floor_policy,
        Some(first_floor.id.clone())
    );
    assert_eq!(
        current.protected_floor_policies[&first_floor.id],
        first_floor
    );
    assert_eq!(protected(&current, "granary"), (37, 63));
    assert!(
        current.completion_leases.acquisitions.is_empty(),
        "the operation is token-only"
    );

    // An access grant is consent to draw on stock, not custody: the grantee
    // cannot set the grantor's floor.
    assert_eq!(
        resource_access_grant_status(&run.canwu, &holder(GRANTEE), &grant_id())
            .expect("the grantee reads its grant")
            .status,
        ResourceAccessGrantStatusV1::Active
    );
    run.refused(
        GRANTEE,
        at,
        set_floor(
            "fixture:floor:grantee",
            &current,
            "granary",
            Some(&floor(1)),
            true,
        ),
        ErrorCode::InvalidAuthority,
    );

    // A resend is not a second change, and the key belongs to its request.
    let resent = run.settled(CUSTODIAN, at, first.clone());
    assert_eq!(resent, outcome);
    let after_resend = state(&run.canwu);
    assert_eq!(after_resend.accounts, current.accounts);
    assert_eq!(after_resend.outcomes, current.outcomes);
    let ResourceOperationRequestV1::SetProtectedFloor(mut other) = first else {
        unreachable!("the request sets a floor")
    };
    let other_floor = floor(38);
    other.policy = Some(other_floor.id.clone());
    other.install = Some(other_floor);
    run.refused(
        CUSTODIAN,
        at,
        ResourceOperationRequestV1::SetProtectedFloor(other),
        ErrorCode::IdempotencyConflict,
    );

    // A body that is not content-addressed, a body the account is not
    // pointed at, and a stale account revision are recorded rejections.
    let mut altered = floor(40);
    altered.floor = 41;
    let free = authored("free", 5);
    let stale = ResourceOperationRequestV1::SetProtectedFloor(ResourceProtectedFloorRequestV1 {
        operation_key: key("fixture:floor:stale"),
        account: account("granary"),
        expected_account_revision: ResourceRevision::INITIAL,
        policy: Some(floor(40).id),
        install: Some(floor(40)),
    });
    let ResourceOperationRequestV1::SetProtectedFloor(mut elsewhere) = set_floor(
        "fixture:floor:elsewhere",
        &current,
        "granary",
        Some(&floor(40)),
        true,
    ) else {
        unreachable!("the request sets a floor")
    };
    elsewhere.policy = Some(rung.id.clone());
    for (request, code) in [
        (
            set_floor(
                "fixture:floor:altered",
                &current,
                "granary",
                Some(&altered),
                true,
            ),
            "invalid_definition",
        ),
        (
            set_floor("fixture:floor:free", &current, "granary", Some(&free), true),
            "invalid_definition",
        ),
        (
            ResourceOperationRequestV1::SetProtectedFloor(elsewhere),
            "invalid_definition",
        ),
        (stale, "version_conflict"),
    ] {
        let before = state(&run.canwu);
        let outcome = run.settled(CUSTODIAN, at, request);
        assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
        assert_eq!(outcome.rejection_code.as_deref(), Some(code));
        let after = state(&run.canwu);
        assert_eq!(after.accounts, before.accounts);
        assert_eq!(
            after.protected_floor_policies,
            before.protected_floor_policies
        );
    }

    // A demand that cites the first floor keeps that revision in the root.
    let pinned = demand(
        &current,
        "pinned",
        CUSTODIAN,
        10,
        day(200),
        Some(&first_floor),
    );
    let submitted = run.settled(CUSTODIAN, at, submit(pinned));
    assert_eq!(submitted.status, ResourceOperationStatus::Applied);

    // The floor changes every day for a month. The root holds the authored
    // rung, the revision the demand cites, and the floor in force: yesterday's
    // revision is retired as today's is installed.
    let mut today = first_floor.clone();
    for number in 1..=30_i64 {
        let at = day(number);
        today = floor(100 + u64::try_from(number).expect("day"));
        let outcome = run.settled(
            CUSTODIAN,
            at,
            set_floor(
                &format!("fixture:floor:day-{number}"),
                &state(&run.canwu),
                "granary",
                Some(&today),
                true,
            ),
        );
        assert_eq!(outcome.status, ResourceOperationStatus::Applied);
        assert_eq!(
            floor_ids(&state(&run.canwu)),
            BTreeSet::from([rung.id.clone(), first_floor.id.clone(), today.id.clone()]),
            "day {number}"
        );
    }

    // Once the demand has left the root, the revision it cited is retired by
    // the next floor change.
    let at = day(31);
    let revision = state(&run.canwu).demands[&demand_id("pinned")].revision;
    let cancelled = run.settled(
        CUSTODIAN,
        at,
        ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:cancel:pinned"),
            demand: demand_id("pinned"),
            expected_demand_revision: revision,
        }),
    );
    assert_eq!(cancelled.status, ResourceOperationStatus::Applied);
    run.archive_all();
    let archived = state(&run.canwu);
    assert!(!archived.demands.contains_key(&demand_id("pinned")));
    assert!(
        archived
            .protected_floor_policies
            .contains_key(&first_floor.id)
    );
    let yesterday = today;
    let today = floor(7);
    let outcome = run.settled(
        CUSTODIAN,
        at,
        set_floor(
            "fixture:floor:day-31",
            &archived,
            "granary",
            Some(&today),
            true,
        ),
    );
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(
        floor_ids(&state(&run.canwu)),
        BTreeSet::from([rung.id.clone(), today.id.clone()])
    );

    // An archive commit settles before the other packets of its boundary, so
    // a floor install queued beside the commit already finds the demand gone.
    // A second demand pins the floor in force through one more change and is
    // cancelled; then one boundary settles the commit that archives it and
    // the next install, which retires both earlier revisions.
    let again = demand(
        &state(&run.canwu),
        "pinned-again",
        CUSTODIAN,
        10,
        day(200),
        Some(&today),
    );
    let submitted = run.settled(CUSTODIAN, at, submit(again));
    assert_eq!(submitted.status, ResourceOperationStatus::Applied);
    let replaced = floor(8);
    let outcome = run.settled(
        CUSTODIAN,
        at,
        set_floor(
            "fixture:floor:replaced",
            &state(&run.canwu),
            "granary",
            Some(&replaced),
            true,
        ),
    );
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    let revision = state(&run.canwu).demands[&demand_id("pinned-again")].revision;
    let cancelled = run.settled(
        CUSTODIAN,
        at,
        ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:cancel:pinned-again"),
            demand: demand_id("pinned-again"),
            expected_demand_revision: revision,
        }),
    );
    assert_eq!(cancelled.status, ResourceOperationStatus::Applied);
    let hot = state(&run.canwu);
    assert_eq!(
        floor_ids(&hot),
        BTreeSet::from([rung.id.clone(), today.id.clone(), replaced.id.clone()])
    );
    let beside = floor(9);
    let admitted = run.admit(
        CUSTODIAN,
        at,
        set_floor(
            "fixture:floor:beside-commit",
            &hot,
            "granary",
            Some(&beside),
            true,
        ),
    );
    assert!(
        matches!(admitted, CommandAttemptOutcome::Accepted { .. }),
        "{admitted:?}"
    );
    assert_eq!(state(&run.canwu), hot, "admission settles nothing");
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), &run.archive)
        .expect("prepare archive commit");
    let receipt = enqueue_resource_archive(&mut run.canwu, &prepared, &run.archive)
        .expect("enqueue commit behind the admitted install");
    settle(&mut run.canwu, at);
    let current = state(&run.canwu);
    assert_eq!(
        current
            .archive_maintenance_receipts
            .values()
            .find(|settled| settled.retention_handle_id == receipt.retention_handle_id)
            .map(|settled| settled.disposition),
        Some(ResourceArchiveMaintenanceDispositionV1::Applied),
        "the commit applied to the root it was prepared from"
    );
    assert!(!current.demands.contains_key(&demand_id("pinned-again")));
    assert_eq!(
        current.outcomes[&key("fixture:floor:beside-commit")].status,
        ResourceOperationStatus::Applied
    );
    assert_eq!(
        floor_ids(&current),
        BTreeSet::from([rung.id.clone(), beside.id.clone()])
    );
    assert_eq!(protected(&current, "granary"), (9, 91));
    finalize_resource_archive_retention(&mut run.canwu, &run.archive, &receipt)
        .expect("finalize retention");
    settle(&mut run.canwu, at);

    // A content-addressed revision is named with its body. Its id alone is
    // refused whether the root holds the revision or has retired it, so the
    // answer tells a custodian nothing about what other accounts protect.
    // Sent with its body a retired revision returns unchanged: its identity
    // is its content.
    for (operation, named) in [
        ("fixture:floor:by-id-retired", &yesterday),
        ("fixture:floor:by-id-held", &beside),
    ] {
        let by_id = run.settled(
            CUSTODIAN,
            at,
            set_floor(operation, &state(&run.canwu), "granary", Some(named), false),
        );
        assert_eq!(
            by_id.rejection_code.as_deref(),
            Some("invalid_definition"),
            "{operation}"
        );
    }
    let by_body = run.settled(
        CUSTODIAN,
        at,
        set_floor(
            "fixture:floor:by-body",
            &state(&run.canwu),
            "granary",
            Some(&yesterday),
            true,
        ),
    );
    assert_eq!(by_body.status, ResourceOperationStatus::Applied);
    let current = state(&run.canwu);
    assert_eq!(current.protected_floor_policies[&yesterday.id], yesterday);
    assert_eq!(
        floor_ids(&current),
        BTreeSet::from([rung.id.clone(), yesterday.id.clone()])
    );

    // An authored revision is never retired: the custodian clears its floor,
    // which retires the content-addressed one, and points at the rung by id.
    let cleared = run.settled(
        CUSTODIAN,
        at,
        set_floor("fixture:floor:clear", &current, "granary", None, false),
    );
    assert_eq!(cleared.status, ResourceOperationStatus::Applied);
    let current = state(&run.canwu);
    assert_eq!(floor_ids(&current), BTreeSet::from([rung.id.clone()]));
    assert_eq!(protected(&current, "granary"), (0, 100));
    let ladder = run.settled(
        CUSTODIAN,
        at,
        set_floor(
            "fixture:floor:rung",
            &current,
            "granary",
            Some(&rung),
            false,
        ),
    );
    assert_eq!(ladder.status, ResourceOperationStatus::Applied);
    assert_eq!(protected(&state(&run.canwu), "granary"), (50, 50));

    // An operation key belongs to the first request that settles under it.
    // A second command with the same key and another floor, admitted in the
    // boundary that settles the first, passes admission because the first
    // outcome is not in the root yet. Its packet then settles nothing: the
    // boundary does not fail, nor does any later one, the first floor
    // stands, and the outcome under the key names the request that owns it.
    let contested = key("fixture:floor:contested");
    let current = state(&run.canwu);
    let earlier = set_floor(
        "fixture:floor:contested",
        &current,
        "granary",
        Some(&floor(11)),
        true,
    );
    let later = set_floor(
        "fixture:floor:contested",
        &current,
        "granary",
        Some(&floor(12)),
        true,
    );
    for request in [&earlier, &later] {
        let admitted = run.admit(CUSTODIAN, at, request.clone());
        assert!(
            matches!(admitted, CommandAttemptOutcome::Accepted { .. }),
            "{admitted:?}"
        );
    }
    settle(&mut run.canwu, at);
    settle(&mut run.canwu, at);
    let current = state(&run.canwu);
    assert_eq!(
        current.outcomes[&contested].request_digest,
        earlier.request_digest().expect("request digest")
    );
    assert_ne!(
        current.outcomes[&contested].request_digest,
        later.request_digest().expect("request digest")
    );
    assert_eq!(
        floor_ids(&current),
        BTreeSet::from([rung.id.clone(), floor(11).id])
    );
    assert_eq!(protected(&current, "granary"), (11, 89));

    // Both admitted in one boundary and settled in the next: the first
    // command keeps its effect and the second settles nothing.
    let together = key("fixture:floor:together");
    let earlier = set_floor(
        "fixture:floor:together",
        &current,
        "granary",
        Some(&floor(13)),
        true,
    );
    let later = set_floor(
        "fixture:floor:together",
        &current,
        "granary",
        Some(&floor(14)),
        true,
    );
    run.enqueue(CUSTODIAN, at, earlier.clone(), 0);
    run.enqueue(CUSTODIAN, at, later, 1);
    settle(&mut run.canwu, at);
    let attempts = run.canwu.command_attempts();
    assert!(
        attempts[attempts.len() - 2..]
            .iter()
            .all(|attempt| matches!(attempt.outcome, CommandAttemptOutcome::Accepted { .. })),
        "both commands are admitted before either settles"
    );
    settle(&mut run.canwu, at);
    let current = state(&run.canwu);
    assert_eq!(
        current.outcomes[&together].request_digest,
        earlier.request_digest().expect("request digest")
    );
    assert_eq!(protected(&current, "granary"), (13, 87));
    // The run goes on.
    let cleared = run.settled(
        CUSTODIAN,
        at,
        set_floor("fixture:floor:after", &current, "granary", None, false),
    );
    assert_eq!(cleared.status, ResourceOperationStatus::Applied);

    // Restore audits the root and its archive; exact replay settles the same.
    let snapshot = run.canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json_with_archive_store(
        &snapshot,
        &plugins,
        Rc::new(run.archive.clone()),
    )
    .expect("restore");
    let replayed = replay_resource_from_journal_with_archive_store(
        &plugins,
        &run.canwu.replay_journal(),
        Rc::new(run.archive.clone()),
    )
    .expect("exact replay");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
    assert_eq!(
        replayed.boundary_head_hash(),
        run.canwu.boundary_head_hash()
    );
}

#[test]
fn a_root_holds_a_bounded_number_of_floor_revisions() {
    let mut state = world();
    // An identity under the content prefix is derived from its content, in a
    // scenario as at runtime.
    let mut altered = floor(5);
    altered.floor = 6;
    assert!(matches!(
        state.install_protected_floor_policy(altered),
        Err(ResourceError::InvalidDefinition(_))
    ));
    for index in 1..MAX_PROTECTED_FLOOR_POLICY_REVISIONS {
        state
            .install_protected_floor_policy(authored(&format!("rung-{index}"), 1))
            .expect("authored rung");
    }
    assert_eq!(
        state.protected_floor_policies.len(),
        MAX_PROTECTED_FLOOR_POLICY_REVISIONS - 1
    );

    // The last place is taken by a runtime install, and replacing that floor
    // at the bound still settles: the revision it replaces is retired first.
    let request = set_floor(
        "fixture:floor:one",
        &state,
        "granary",
        Some(&floor(1)),
        true,
    );
    applied(&mut state, request);
    let request = set_floor(
        "fixture:floor:two",
        &state,
        "granary",
        Some(&floor(2)),
        true,
    );
    applied(&mut state, request);
    assert_eq!(
        state.protected_floor_policies.len(),
        MAX_PROTECTED_FLOOR_POLICY_REVISIONS
    );
    assert!(!state.protected_floor_policies.contains_key(&floor(1).id));
    state.validate().expect("a root at the bound");

    // Past the bound a scenario install is refused and a runtime install is a
    // recorded rejection that changes nothing.
    assert!(matches!(
        state.install_protected_floor_policy(authored("one-too-many", 1)),
        Err(ResourceError::LimitExceeded(_))
    ));
    let before = state.clone();
    let request = set_floor(
        "fixture:floor:three",
        &state,
        "annex",
        Some(&floor(3)),
        true,
    );
    rejected(&mut state, request, "limit_exceeded");
    assert_eq!(state.accounts, before.accounts);
    assert_eq!(
        state.protected_floor_policies,
        before.protected_floor_policies
    );

    // Accounts share a revision of equal content, and it is retired only
    // when the last of them lets go.
    let request = set_floor(
        "fixture:floor:shared",
        &state,
        "annex",
        Some(&floor(2)),
        true,
    );
    applied(&mut state, request);
    let request = set_floor("fixture:floor:release-one", &state, "granary", None, false);
    applied(&mut state, request);
    assert!(state.protected_floor_policies.contains_key(&floor(2).id));
    let request = set_floor("fixture:floor:release-two", &state, "annex", None, false);
    applied(&mut state, request);
    assert_eq!(
        state.protected_floor_policies.len(),
        MAX_PROTECTED_FLOOR_POLICY_REVISIONS - 1
    );
    state.validate().expect("a root below the bound");
}

/// Activates one lease held by the custodian through the single step and
/// returns its certificate.
fn lease(
    state: &mut ResourceState,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: Vec<CompletionLockedTargetV1>,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
    let id = CompletionLeaseAcquisitionId::new(format!("fixture:lease:{suffix}")).expect("lease");
    applied(
        state,
        ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::SingleStepLease(
            SingleStepCompletionLeaseV1 {
                lease: RequestCompletionLeaseV1 {
                    id: id.clone(),
                    operation_key: operation_key.clone(),
                    holder: holder(CUSTODIAN),
                    operation_namespace: NAMESPACE.to_owned(),
                    eligibility_time: at,
                    eligibility_envelope: EligibilityEnvelopeV1::new(
                        targets
                            .iter()
                            .filter_map(|target| match target {
                                CompletionLockedTargetV1::ExternalRecord { version } => {
                                    Some(version.clone())
                                }
                                _ => None,
                            })
                            .collect(),
                        BTreeMap::new(),
                        BTreeSet::new(),
                        Vec::new(),
                        Vec::new(),
                    )
                    .expect("envelope"),
                    recipe: CompletionCapacityRecipeV1 {
                        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                        mutations: 2,
                        reports_per_holder: 0,
                        holders: 0,
                        bytes: 1_024,
                    },
                    expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                },
                grant_id: CompletionCapacityGrantId::new(format!("fixture:lease-grant:{suffix}"))
                    .expect("grant"),
                target_versions: targets,
                current_boundary: 10,
            },
        )),
    );
    state.completion_leases.certificates[&id].clone()
}

fn granary_target(state: &ResourceState) -> CompletionLockedTargetV1 {
    CompletionLockedTargetV1::Account {
        id: account("granary"),
        revision: state.accounts[&account("granary")].revision,
    }
}

/// Everything that reads the floor after it was set, with a reservation, an
/// open demand, and a granted demand already in flight. Returns the outcome
/// of every operation, in order.
fn read_the_floor(
    state: &mut ResourceState,
    reserve: &ProtectedFloorPolicyRevision,
) -> Vec<ResourceOperationOutcome> {
    let mut outcomes = Vec::new();
    assert_eq!(protected(state, "granary"), (60, 20));

    // The open demand cites the floor it was admitted under, so its pass is
    // a recorded rejection until it is amended to the floor in force.
    let request = allocate("fixture:allocate:stale-demand", state, minute(5));
    outcomes.push(rejected(state, request, "version_conflict"));
    let mut waiting = state.demands[&demand_id("waiting")].clone();
    let expected_demand_revision = waiting.revision;
    waiting.protected_floor_policy = Some(reserve.id.clone());
    outcomes.push(applied(
        state,
        ResourceOperationRequestV1::AmendDemand(ResourceAmendDemandRequestV1 {
            operation_key: key("fixture:amend:waiting"),
            expected_demand_revision,
            replacement: waiting,
        }),
    ));
    // Allocation honors the floor: 20 of the 100 units are reserved and 60
    // protected, so 15 of the remaining 20 go to the amended demand. The
    // granted demand still cites the earlier floor: the grantor's account
    // supplies it nothing, which does not fail the pass.
    let request = allocate("fixture:allocate:under-floor", state, minute(5));
    let pass = applied(state, request);
    assert_eq!(pass.quantity, 15);
    outcomes.push(pass);
    assert_eq!(
        state.demands[&demand_id("granted")].status,
        DemandStatus::RejectedMinimum
    );
    assert_eq!(protected(state, "granary"), (60, 5));

    // A granted demand that cites the floor in force draws under it.
    let request = submit(demand(
        state,
        "granted-again",
        GRANTEE,
        5,
        minute(5),
        Some(reserve),
    ));
    outcomes.push(applied(state, request));
    let request = allocate("fixture:allocate:granted", state, minute(5));
    let pass = applied(state, request);
    assert_eq!(pass.quantity, 5);
    outcomes.push(pass);
    assert_eq!(protected(state, "granary"), (60, 0));

    // A demand of an override class the revision names draws below the floor.
    let mut emergency = demand(state, "emergency", CUSTODIAN, 30, minute(5), Some(reserve));
    emergency.protection_override_class = Some("emergency".to_owned());
    outcomes.push(applied(state, submit(emergency)));
    let request = allocate("fixture:allocate:override", state, minute(5));
    let pass = applied(state, request);
    assert_eq!(pass.quantity, 30);
    outcomes.push(pass);

    // The reservation made before the floor rose is still the custodian's to
    // move: its debit locks the account revision the floor change left.
    let leg = state
        .allocation_legs
        .values()
        .find(|leg| leg.demand == demand_id("early"))
        .cloned()
        .expect("early leg");
    let start_key = key("fixture:start:early");
    let targets = vec![
        granary_target(state),
        CompletionLockedTargetV1::AllocationLeg {
            id: leg.id.clone(),
            revision: leg.revision,
        },
        CompletionLockedTargetV1::Demand {
            id: leg.demand.clone(),
            revision: leg.demand_revision,
        },
    ];
    let certificate = lease(state, "start-early", &start_key, targets, minute(6));
    let expected_account_revision = state.accounts[&account("granary")].revision;
    outcomes.push(applied(
        state,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: ResourceTransferId::new("fixture:transfer:early").expect("transfer"),
            allocation: (&leg).into(),
            expected_account_revision,
            destination: Some(account("annex")),
            at: minute(6),
            completion_certificate: certificate,
        }),
    ));
    assert_eq!(state.accounts[&account("granary")].balance, 80);

    // A loss may take protected stock only when it says so; an outflow that
    // does not say so is refused by the floor.
    let cause = evidence("spoilage");
    let loss_key = key("fixture:loss:protected");
    let targets = vec![
        granary_target(state),
        CompletionLockedTargetV1::ExternalRecord {
            version: cause.clone(),
        },
    ];
    let certificate = lease(state, "loss-protected", &loss_key, targets, minute(7));
    let expected_account_revision = state.accounts[&account("granary")].revision;
    outcomes.push(applied(
        state,
        ResourceOperationRequestV1::RecordLoss(ResourceAccountLossRequestV1 {
            operation_key: loss_key,
            loss_id: ResourceLossId::new("fixture:loss:protected").expect("loss"),
            account: account("granary"),
            expected_account_revision,
            quantity: 10,
            cause: EvidenceRef::DomainRecordVersion(cause),
            allow_protected: true,
            at: minute(7),
            completion_certificate: certificate,
        }),
    ));
    let order = evidence("order");
    let outflow_key = key("fixture:outflow:refused");
    let targets = vec![
        granary_target(state),
        CompletionLockedTargetV1::ExternalRecord {
            version: order.clone(),
        },
    ];
    let certificate = lease(state, "outflow-refused", &outflow_key, targets, minute(8));
    let expected_account_revision = state.accounts[&account("granary")].revision;
    let request = ResourceOperationRequestV1::ExternalOutflow(ResourceExternalOutflowRequestV1 {
        operation_key: outflow_key,
        account: account("granary"),
        expected_account_revision,
        quantity: 10,
        allow_protected: false,
        authority_evidence: order,
        at: minute(8),
        completion_certificate: certificate,
    });
    outcomes.push(rejected(state, request, "protected_floor"));
    assert_eq!(state.accounts[&account("granary")].balance, 70);

    // The floor changes again with all of that in flight. The demands still
    // cite the first revision, so it stays beside the new one.
    let lower = floor(10);
    let request = set_floor("fixture:floor:lower", state, "granary", Some(&lower), true);
    outcomes.push(applied(state, request));
    assert_eq!(
        floor_ids(state),
        BTreeSet::from([reserve.id.clone(), lower.id.clone()])
    );
    assert_eq!(protected(state, "granary"), (10, 10));
    let cause = evidence("vermin");
    let loss_key = key("fixture:loss:available");
    let targets = vec![
        granary_target(state),
        CompletionLockedTargetV1::ExternalRecord {
            version: cause.clone(),
        },
    ];
    let certificate = lease(state, "loss-available", &loss_key, targets, minute(9));
    let expected_account_revision = state.accounts[&account("granary")].revision;
    outcomes.push(applied(
        state,
        ResourceOperationRequestV1::RecordLoss(ResourceAccountLossRequestV1 {
            operation_key: loss_key,
            loss_id: ResourceLossId::new("fixture:loss:available").expect("loss"),
            account: account("granary"),
            expected_account_revision,
            quantity: 5,
            cause: EvidenceRef::DomainRecordVersion(cause),
            allow_protected: false,
            at: minute(9),
            completion_certificate: certificate,
        }),
    ));
    assert_eq!(state.accounts[&account("granary")].balance, 65);
    state.validate().expect("state after every reader");
    outcomes
}

#[test]
fn a_runtime_installed_floor_reads_exactly_as_a_scenario_installed_one() {
    // In flight before the floor is set: a reservation of 20, an open demand
    // of 15, and a granted demand of 10, all admitted with no floor.
    let mut base = world();
    for (name, requester, requested, due_at) in [
        ("early", CUSTODIAN, 20, SimTime::EPOCH),
        ("waiting", CUSTODIAN, 15, minute(5)),
        ("granted", GRANTEE, 10, minute(5)),
    ] {
        let request = submit(demand(&base, name, requester, requested, due_at, None));
        applied(&mut base, request);
    }
    let request = allocate("fixture:allocate:early", &base, SimTime::EPOCH);
    assert_eq!(applied(&mut base, request).quantity, 20);

    let reserve = ProtectedFloorPolicyRevision::content_addressed(
        ResourceRevision::INITIAL,
        60,
        BTreeSet::from(["emergency".to_owned()]),
    )
    .expect("reserve floor");
    let request = |state: &ResourceState| {
        set_floor(
            "fixture:floor:reserve",
            state,
            "granary",
            Some(&reserve),
            true,
        )
    };

    // One root gets the revision from its scenario, and the custodian's
    // request finds it installed. The other gets it from that request. The
    // roots are then equal: nothing records how the revision arrived.
    let mut scenario = base.clone();
    scenario
        .install_protected_floor_policy(reserve.clone())
        .expect("scenario install");
    let installed = request(&scenario);
    applied(&mut scenario, installed);
    let mut runtime = base.clone();
    let installed = request(&runtime);
    applied(&mut runtime, installed);
    assert_eq!(runtime, scenario);

    // A third root holds an authored revision of the same content, as a
    // scenario's ladder does, and points at it by id.
    let rung = ProtectedFloorPolicyRevision {
        id: ProtectedFloorPolicyRevisionId::new("fixture:floor:reserve-rung").expect("floor"),
        semantic_digest: "a".repeat(64),
        ..reserve.clone()
    };
    let mut ladder = base;
    ladder
        .install_protected_floor_policy(rung.clone())
        .expect("authored rung");
    let pointed = set_floor(
        "fixture:floor:reserve",
        &ladder,
        "granary",
        Some(&rung),
        false,
    );
    applied(&mut ladder, pointed);

    // Every reader finds the same on the two roots that share the revision,
    // operation by operation, and they are still equal at the end.
    let scenario_outcomes = read_the_floor(&mut scenario, &reserve);
    let runtime_outcomes = read_the_floor(&mut runtime, &reserve);
    assert_eq!(runtime_outcomes, scenario_outcomes);
    assert_eq!(runtime, scenario);

    // Against the authored revision every operation has the same effect.
    // Only the digests differ, because the demands cite another identity.
    let effects = |outcomes: &[ResourceOperationOutcome]| {
        outcomes
            .iter()
            .map(|outcome| {
                (
                    outcome.kind,
                    outcome.status,
                    outcome.quantity,
                    outcome.remainder,
                    outcome.rejection_code.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let ladder_outcomes = read_the_floor(&mut ladder, &rung);
    assert_eq!(effects(&ladder_outcomes), effects(&runtime_outcomes));
    assert_eq!(ladder.accounts, runtime.accounts);
    assert_eq!(ladder.conservation, runtime.conservation);
    for name in ["granary", "annex"] {
        assert_eq!(protected(&ladder, name), protected(&runtime, name));
    }
}
