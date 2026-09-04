use canwu_core::{
    DecisionRequestId, DecisionTicketId, DecisionTraceId, DomainRecordKind, DomainRecordRef,
    DomainRecordVersionRef, DomainRecordVersionSource,
};
use canwu_sim::{
    ArchiveStoreOutcome, BoundaryRequest, DecisionArchiveBlob, DecisionArchiveProvider,
    DecisionArchiveStore, DecisionArchiveStoreOutcome, DecisionHistoryKey, DecisionHistoryLocation,
    DecisionIngressRequest, DecisionMutation, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordSchema, ErrorCode,
    MaintenanceDisposition, OwnerAuthorizedMaintenanceRequest, OwnerAuthorizedMutation,
    OwnerAuthorizedParticipantDraft, OwnerAuthorizedParticipantProposal,
    OwnerAuthorizedParticipantRole, OwnerAuthorizedRecordExpectation,
    PLUGIN_DESCRIPTOR_FORMAT_VERSION, PayloadSchema, PersistentDomainRecordStore,
    PreparedStateDelta, Scenario, Simulation, SimulationPlugin, SimulationView, StatePageBlob,
    StatePageProvider, StatePageRetentionLedger, StatePageRetentionPhase, StatePageStore,
    StateVisibility, SystemCadence, canonical_hash, prepare_state_delta, state_page_id,
    verify_state_delta,
};
use canwu_time::SimTime;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct PageStore(RefCell<BTreeMap<String, StatePageBlob>>);

#[derive(Default)]
struct DecisionStore(RefCell<BTreeMap<String, DecisionArchiveBlob>>);

struct RetirementFixturePlugin {
    name: &'static str,
    namespace: &'static str,
    kind: &'static str,
    semantic_hash: &'static str,
    resolves: Option<&'static str>,
}

impl SimulationPlugin for RetirementFixturePlugin {
    fn name(&self) -> &str {
        self.name
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &str {
        self.semantic_hash
    }

    fn register(
        &self,
        registrar: &mut canwu_sim::PluginRegistrar<'_>,
    ) -> Result<(), canwu_sim::CanwuError> {
        let mut schema = DomainRecordSchema::new(
            DomainRecordKind::new(self.namespace, self.kind),
            DomainRecordClass::Record,
        );
        schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(schema)?;
        if let Some(target_namespace) = self.resolves {
            registrar.register_maintenance_dependency_resolver(target_namespace)?;
        }
        registrar.register_owner_authorized_maintenance_participant(
            owner_authorized_fixture_participant,
        )?;
        Ok(())
    }
}

fn owner_authorized_fixture_participant(
    view: &SimulationView<'_>,
    request: &OwnerAuthorizedMaintenanceRequest,
    role: OwnerAuthorizedParticipantRole,
) -> Result<OwnerAuthorizedParticipantDraft, canwu_sim::CanwuError> {
    match role {
        OwnerAuthorizedParticipantRole::TargetOwner => {
            let target = view.domain_record(&request.target.record)?.ok_or_else(|| {
                canwu_sim::CanwuError::new(ErrorCode::InvalidDomainRecord, "target missing")
            })?;
            Ok(OwnerAuthorizedParticipantDraft {
                plugin: target.owner.clone(),
                role,
                accepted: true,
                rejection_reason: None,
                expected_records: vec![request.target.clone()],
                mutations: vec![OwnerAuthorizedMutation {
                    mutation: DomainRecordMutation::Retire {
                        record: request.target.record.clone(),
                        expected_version: request.target.version,
                        successor: None,
                    },
                    visibility: StateVisibility::NextBoundary,
                    summary: "Retire the owner-selected dormant generation".to_owned(),
                }],
            })
        }
        OwnerAuthorizedParticipantRole::DependentOwner => {
            let reference = serde_json::from_value::<DomainRecordRef>(
                request
                    .payload
                    .get("dependent_record")
                    .cloned()
                    .ok_or_else(|| {
                        canwu_sim::CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "dependent record is required",
                        )
                    })?,
            )
            .map_err(|error| {
                canwu_sim::CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid dependent record: {error}"),
                )
            })?;
            let record = view.domain_record(&reference)?.ok_or_else(|| {
                canwu_sim::CanwuError::new(ErrorCode::InvalidDomainRecord, "dependency missing")
            })?;
            let mut payload = record.payload.clone();
            payload["acknowledged"] = serde_json::json!(true);
            let mut draft = DomainRecordDraft::new(reference.clone(), payload);
            draft.references.clone_from(&record.references);
            Ok(OwnerAuthorizedParticipantDraft {
                plugin: record.owner.clone(),
                role,
                accepted: true,
                rejection_reason: None,
                expected_records: vec![OwnerAuthorizedRecordExpectation {
                    record: reference,
                    version: record.version,
                }],
                mutations: vec![OwnerAuthorizedMutation {
                    mutation: DomainRecordMutation::Update {
                        record: draft,
                        expected_version: record.version,
                    },
                    visibility: StateVisibility::NextBoundary,
                    summary: "Acknowledge the resolved law dependency".to_owned(),
                }],
            })
        }
    }
}

fn owner_authorized_fixture() -> (
    Simulation,
    RetirementFixturePlugin,
    RetirementFixturePlugin,
    DomainRecordRef,
    DomainRecordRef,
) {
    let culture_ref = DomainRecordRef {
        kind: DomainRecordKind::new("culture.fixture", "target"),
        id: "rights-generation-1".to_owned(),
    };
    let law_ref = DomainRecordRef {
        kind: DomainRecordKind::new("law.fixture", "dependency"),
        id: "rights-law-dependency".to_owned(),
    };
    let mut scenario = Scenario::new(SimTime::EPOCH, Vec::new());
    scenario.domain_records = vec![
        DomainRecord {
            reference: culture_ref.clone(),
            owner: "culture-owner".to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: serde_json::json!({ "active": true }),
            references: Vec::new(),
        },
        DomainRecord {
            reference: law_ref.clone(),
            owner: "law-dependent".to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: serde_json::json!({ "live_level": false, "acknowledged": false }),
            references: Vec::new(),
        },
    ];
    let culture = RetirementFixturePlugin {
        name: "culture-owner",
        namespace: "culture.fixture",
        kind: "target",
        semantic_hash: "1111111111111111111111111111111111111111111111111111111111111111",
        resolves: None,
    };
    let law = RetirementFixturePlugin {
        name: "law-dependent",
        namespace: "law.fixture",
        kind: "dependency",
        semantic_hash: "2222222222222222222222222222222222222222222222222222222222222222",
        resolves: Some("culture.fixture"),
    };
    let simulation =
        Simulation::new_with_plugins(41, scenario, &[&culture, &law]).expect("fixture simulation");
    (simulation, culture, law, culture_ref, law_ref)
}

fn owner_authorized_retirement_draft(
    simulation: &Simulation,
    culture_ref: &DomainRecordRef,
    law_ref: &DomainRecordRef,
) -> OwnerAuthorizedMaintenanceRequest {
    let expectation = OwnerAuthorizedRecordExpectation {
        record: culture_ref.clone(),
        version: 1,
    };
    OwnerAuthorizedMaintenanceRequest {
        request_id: "retire-rights-generation-1".to_owned(),
        target: expectation.clone(),
        requested_at: simulation.time(),
        payload: serde_json::json!({
            "dependent_record": law_ref,
            "attempted_foreign_mutation": {
                "delete": culture_ref,
                "expected_version": 1
            }
        }),
    }
}

impl DecisionArchiveProvider for DecisionStore {
    fn load_decision_archive(
        &self,
        locator: &str,
    ) -> Result<Option<DecisionArchiveBlob>, canwu_sim::DecisionError> {
        Ok(self.0.borrow().get(locator).cloned())
    }
}

impl DecisionArchiveStore for DecisionStore {
    fn store_decision_archive(
        &self,
        blob: &DecisionArchiveBlob,
    ) -> Result<DecisionArchiveStoreOutcome, canwu_sim::DecisionError> {
        let locator = blob.content_id()?;
        let mut blobs = self.0.borrow_mut();
        if let Some(existing) = blobs.get(&locator) {
            return if existing == blob {
                Ok(DecisionArchiveStoreOutcome::AlreadyStored)
            } else {
                Err(canwu_sim::DecisionError::new(
                    canwu_sim::DecisionErrorCode::InvalidDecision,
                    "decision archive locator has conflicting bytes",
                ))
            };
        }
        blobs.insert(locator, blob.clone());
        Ok(DecisionArchiveStoreOutcome::Stored)
    }
}

impl StatePageProvider for PageStore {
    fn load_state_page(
        &self,
        page_id: &str,
    ) -> Result<Option<StatePageBlob>, canwu_sim::CanwuError> {
        Ok(self.0.borrow().get(page_id).cloned())
    }
}

impl DecisionArchiveProvider for PageStore {
    fn load_decision_archive(
        &self,
        _locator: &str,
    ) -> Result<Option<DecisionArchiveBlob>, canwu_sim::DecisionError> {
        Ok(None)
    }

    fn load_decision_archive_bucket_page(
        &self,
        page_id: &str,
    ) -> Result<Option<canwu_sim::DecisionArchiveBucketPage>, canwu_sim::DecisionError> {
        self.0
            .borrow()
            .get(page_id)
            .map(|page| {
                serde_json::from_slice(&page.bytes).map_err(|error| {
                    canwu_sim::DecisionError::new(
                        canwu_sim::DecisionErrorCode::DecisionHistoryUnavailable,
                        format!("invalid decision bucket page: {error}"),
                    )
                })
            })
            .transpose()
    }
}

impl StatePageStore for PageStore {
    fn store_state_page(
        &self,
        page: &StatePageBlob,
    ) -> Result<ArchiveStoreOutcome, canwu_sim::CanwuError> {
        page.validate()?;
        let mut pages = self.0.borrow_mut();
        if let Some(existing) = pages.get(&page.page_id) {
            return if existing == page {
                Ok(ArchiveStoreOutcome::AlreadyPresent)
            } else {
                Err(canwu_sim::CanwuError::new(
                    ErrorCode::InvalidArchive,
                    "state page ID has conflicting bytes",
                ))
            };
        }
        pages.insert(page.page_id.clone(), page.clone());
        Ok(ArchiveStoreOutcome::Stored)
    }
}

#[test]
fn format8_snapshot_rejects_format7_and_exposes_page_contract() {
    let (simulation, _) = Simulation::demo(17).expect("demo should initialize");
    let mut wire: serde_json::Value = serde_json::from_str(
        &simulation
            .snapshot_json()
            .expect("snapshot should serialize"),
    )
    .expect("snapshot JSON should be an object");
    assert_eq!(wire["snapshot_format_version"], serde_json::json!(8));
    wire["snapshot_format_version"] = serde_json::json!(7);
    let Err(error) = Simulation::from_snapshot_json(&wire.to_string()) else {
        panic!("format 7 rejected");
    };
    assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

    let page = StatePageBlob::new(b"canonical state page".to_vec()).expect("page");
    assert_eq!(page.page_id, state_page_id(b"canonical state page"));
    let store = PageStore::default();
    assert_eq!(
        store.store_state_page(&page).expect("store"),
        ArchiveStoreOutcome::Stored
    );
    assert_eq!(
        store.store_state_page(&page).expect("idempotent store"),
        ArchiveStoreOutcome::AlreadyPresent
    );
    let prepared =
        prepare_state_delta(&"a".repeat(64), &"b".repeat(64), vec![page.clone()]).expect("delta");
    verify_state_delta(&prepared, &store).expect("provider verifies exact bytes");
}

#[test]
fn decision_history_uses_typed_locations() {
    let (simulation, _) = Simulation::demo(19).expect("demo should initialize");
    assert!(matches!(
        simulation.decision_history_location(&DecisionHistoryKey::Ticket(DecisionTicketId::new(1))),
        DecisionHistoryLocation::Absent
    ));
    assert!(matches!(
        simulation
            .decision_history_location(&DecisionHistoryKey::Attempt(DecisionRequestId::new(999))),
        DecisionHistoryLocation::Absent
    ));
    assert!(matches!(
        simulation.decision_history_location(&DecisionHistoryKey::Trace(DecisionTraceId::new(1))),
        DecisionHistoryLocation::Absent
    ));
    assert_eq!(PLUGIN_DESCRIPTOR_FORMAT_VERSION, 1);
    let hot = simulation.decision_hot_state();
    assert_eq!(hot.trace_count, 0);
}

#[test]
fn prepared_state_delta_is_strictly_self_consistent() {
    let page = StatePageBlob::new(vec![42; 32]).expect("page");
    let mut prepared: PreparedStateDelta =
        prepare_state_delta(&"c".repeat(64), &"d".repeat(64), vec![page]).expect("delta");
    prepared.token_hash.replace_range(..1, "0");
    assert_eq!(
        prepared.validate().expect_err("tampered token").code,
        ErrorCode::InvalidArchive
    );
}

#[test]
fn domain_record_store_commits_primary_and_reverse_indexes() {
    let kind = DomainRecordKind::new("format8", "fixture");
    let records = (0..3)
        .map(|ordinal| {
            let reference = DomainRecordRef {
                kind: kind.clone(),
                id: format!("record-{ordinal}"),
            };
            (
                reference.clone(),
                DomainRecord {
                    reference,
                    owner: "format8".to_owned(),
                    class: DomainRecordClass::Record,
                    version: 1,
                    lifecycle: DomainRecordLifecycle::Active,
                    payload: serde_json::json!({ "ordinal": ordinal }),
                    references: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let store = PersistentDomainRecordStore::from_records(records.clone()).expect("store");
    assert_eq!(store.len(), 3);
    assert_eq!(store.roots().primary.len(), 64);
    assert_eq!(store.roots().reverse_references.len(), 64);
    assert_eq!(store.roots().successor_of.len(), 64);
    assert_eq!(store.roots().predecessors_of.len(), 64);

    let (page_roots, pages) = store.state_pages().expect("state pages");
    assert!(!pages.is_empty());
    let provider = PageStore::default();
    for page in &pages {
        provider.store_state_page(page).expect("store state page");
    }
    let rebuilt = PersistentDomainRecordStore::from_state_pages(&page_roots, &provider)
        .expect("reconstruct store");
    assert_eq!(rebuilt.materialize(), records);
    assert_eq!(rebuilt.roots(), store.roots());
}

#[test]
fn one_record_change_emits_only_a_patricia_path() {
    let kind = DomainRecordKind::new("format8", "scale-fixture");
    let mut records = (0..4_096)
        .map(|ordinal| {
            let reference = DomainRecordRef {
                kind: kind.clone(),
                id: format!("record-{ordinal:04}"),
            };
            (
                reference.clone(),
                DomainRecord {
                    reference,
                    owner: "format8".to_owned(),
                    class: DomainRecordClass::Record,
                    version: 1,
                    lifecycle: DomainRecordLifecycle::Active,
                    payload: serde_json::json!({ "ordinal": ordinal }),
                    references: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let original = PersistentDomainRecordStore::from_records(records.clone()).expect("store");
    let (_, original_pages) = original.state_pages().expect("initial pages");
    let provider = PageStore::default();
    for page in &original_pages {
        provider.store_state_page(page).expect("store page");
    }

    let changed = records
        .get_mut(&DomainRecordRef {
            kind,
            id: "record-2048".to_owned(),
        })
        .expect("changed record");
    changed.version = 2;
    changed.payload = serde_json::json!({ "ordinal": 2048, "changed": true });
    let changed = PersistentDomainRecordStore::from_records(records).expect("changed store");
    let (_, delta_pages) = changed
        .missing_state_pages(&provider)
        .expect("path-only pages");
    assert!(
        delta_pages.len() < 64,
        "delta contained {} pages",
        delta_pages.len()
    );
    assert!(delta_pages.len() < original_pages.len() / 100);
}

#[test]
fn paged_checkpoint_round_trips_and_emits_only_missing_pages() {
    let (simulation, _) = Simulation::demo(23).expect("demo should initialize");
    let provider = PageStore::default();
    let prepared = simulation
        .prepare_paged_checkpoint(None, &provider)
        .expect("prepare paged checkpoint");
    assert!(!prepared.delta.new_pages.is_empty());
    prepared
        .store_and_verify(&provider)
        .expect("store paged checkpoint");

    let restored = Simulation::from_paged_checkpoint(&prepared.checkpoint, &provider)
        .expect("restore paged checkpoint");
    assert_eq!(
        restored.authoritative_state_hash().expect("restored hash"),
        simulation.authoritative_state_hash().expect("source hash")
    );
    assert_eq!(restored.checkpoint_hash(), simulation.checkpoint_hash());

    let unchanged = simulation
        .prepare_paged_checkpoint(Some(&prepared.checkpoint), &provider)
        .expect("prepare unchanged delta");
    assert!(unchanged.delta.new_pages.is_empty());
    assert_eq!(unchanged.checkpoint, prepared.checkpoint);

    let portable = simulation
        .portable_paged_checkpoint()
        .expect("portable pages");
    let portable_restored =
        Simulation::from_portable_paged_checkpoint(portable).expect("restore portable pages");
    assert_eq!(
        portable_restored
            .authoritative_state_hash()
            .expect("portable hash"),
        simulation.authoritative_state_hash().expect("source hash")
    );
}

#[test]
fn retention_handoff_survives_restart_and_interlocks_gc() {
    let (simulation, _) = Simulation::demo(29).expect("demo should initialize");
    let store = PageStore::default();
    let prepared = simulation
        .prepare_paged_checkpoint(None, &store)
        .expect("prepare paged checkpoint");
    prepared
        .store_and_verify(&store)
        .expect("store paged checkpoint");
    let mut ledger = StatePageRetentionLedger::default();
    let handle = ledger
        .prepare(&prepared.delta, &store)
        .expect("prepare retention");
    let unrelated = "f".repeat(64);
    assert!(
        !ledger
            .sweep_candidates(store.0.borrow().keys().cloned().chain([unrelated.clone()]))
            .contains(&prepared.delta.new_pages[0].page_id)
    );
    assert!(
        ledger
            .sweep_candidates(store.0.borrow().keys().cloned().chain([unrelated.clone()]))
            .contains(&unrelated)
    );

    let restart = serde_json::to_string(&ledger).expect("persist retention ledger");
    let mut ledger: StatePageRetentionLedger =
        serde_json::from_str(&restart).expect("restore retention ledger");
    ledger
        .verify(&handle, &store)
        .expect("verify retained pages");
    assert_eq!(
        ledger.handles[&handle].phase,
        StatePageRetentionPhase::Verified
    );
    ledger
        .mark_durable_ingress(&handle)
        .expect("durable ingress owns lease");
    ledger.begin_gc_epoch().expect("advance GC epoch");
    ledger.commit(&handle).expect("transfer root lease");
    ledger.validate().expect("committed retention state");
    assert!(
        ledger
            .committed_roots
            .contains_key(&prepared.delta.target_root)
    );
    ledger
        .release_committed_root(&prepared.delta.target_root)
        .expect("release old root");
    assert!(!ledger.handles.contains_key(&handle));
}

#[test]
fn retention_handoff_fault_matrix_preserves_or_releases_pages_by_phase() {
    let (simulation, _) = Simulation::demo(37).expect("demo should initialize");
    let store = PageStore::default();
    let prepared = simulation
        .prepare_paged_checkpoint(None, &store)
        .expect("prepare paged checkpoint");
    prepared.store_and_verify(&store).expect("store pages");
    let page_ids = prepared
        .delta
        .new_pages
        .iter()
        .map(|page| page.page_id.clone())
        .collect::<BTreeSet<_>>();

    for target_phase in [
        StatePageRetentionPhase::Prepared,
        StatePageRetentionPhase::Verified,
        StatePageRetentionPhase::DurableIngress,
        StatePageRetentionPhase::Committed,
    ] {
        let mut ledger = StatePageRetentionLedger::default();
        let handle = ledger
            .prepare(&prepared.delta, &store)
            .expect("prepare retention");
        if target_phase != StatePageRetentionPhase::Prepared {
            ledger.verify(&handle, &store).expect("verify retention");
        }
        if matches!(
            target_phase,
            StatePageRetentionPhase::DurableIngress | StatePageRetentionPhase::Committed
        ) {
            ledger
                .mark_durable_ingress(&handle)
                .expect("durable ingress");
        }
        if target_phase == StatePageRetentionPhase::Committed {
            ledger.commit(&handle).expect("commit retention");
        }

        let encoded = serde_json::to_vec(&ledger).expect("persist fault point");
        let mut restarted: StatePageRetentionLedger =
            serde_json::from_slice(&encoded).expect("restore fault point");
        restarted.validate().expect("validate restarted ledger");
        assert!(page_ids.is_subset(&restarted.reachable_page_ids()));

        if matches!(
            target_phase,
            StatePageRetentionPhase::Prepared | StatePageRetentionPhase::Verified
        ) {
            restarted
                .abandon(&handle)
                .expect("abandon non-durable work");
            assert!(page_ids.is_subset(&restarted.sweep_candidates(page_ids.clone())));
        } else {
            assert!(restarted.abandon(&handle).is_err());
            if target_phase == StatePageRetentionPhase::DurableIngress {
                restarted.commit(&handle).expect("finish durable handoff");
            }
            restarted
                .release_committed_root(&prepared.delta.target_root)
                .expect("release committed root");
            assert!(page_ids.is_subset(&restarted.sweep_candidates(page_ids.clone())));
        }
    }

    let mut missing = StatePageRetentionLedger::default();
    let handle = missing
        .prepare(&prepared.delta, &store)
        .expect("prepare missing-provider case");
    assert!(missing.verify(&handle, &PageStore::default()).is_err());
    assert_eq!(
        missing.handles[&handle].phase,
        StatePageRetentionPhase::Prepared
    );
}

#[test]
fn releasing_one_committed_root_preserves_pages_shared_with_a_newer_root_after_restart() {
    let store = PageStore::default();
    let page = |value: serde_json::Value| {
        StatePageBlob::new(serde_json::to_vec(&value).expect("encode fixture page"))
            .expect("create fixture page")
    };
    let shared = page(serde_json::json!({ "node": "leaf", "value": "shared" }));
    let first_only = page(serde_json::json!({ "node": "leaf", "value": "first" }));
    let second_only = page(serde_json::json!({ "node": "leaf", "value": "second" }));
    let first_root = page(serde_json::json!({
        "node": "branch",
        "left_page": shared.page_id,
        "right_page": first_only.page_id,
    }));
    let second_root = page(serde_json::json!({
        "node": "branch",
        "left_page": shared.page_id,
        "right_page": second_only.page_id,
    }));
    for page in [
        &shared,
        &first_only,
        &second_only,
        &first_root,
        &second_root,
    ] {
        store.store_state_page(page).expect("store fixture page");
    }
    let first = prepare_state_delta(
        &"0".repeat(64),
        &first_root.page_id,
        vec![shared.clone(), first_only, first_root.clone()],
    )
    .expect("prepare first root");
    let second = prepare_state_delta(
        &first.target_root,
        &second_root.page_id,
        vec![second_only, second_root.clone()],
    )
    .expect("prepare second root");

    let mut ledger = StatePageRetentionLedger::default();
    for delta in [&first, &second] {
        let handle = ledger.prepare(delta, &store).expect("prepare retention");
        ledger.verify(&handle, &store).expect("verify retention");
        ledger
            .mark_durable_ingress(&handle)
            .expect("durable retention ingress");
        ledger.commit(&handle).expect("commit retained root");
    }
    let first_pages = ledger.committed_roots[&first.target_root].clone();
    let second_pages = ledger.committed_roots[&second.target_root].clone();
    let shared_pages = first_pages
        .intersection(&second_pages)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(shared_pages, BTreeSet::from([shared.page_id.clone()]));

    let encoded = serde_json::to_vec(&ledger).expect("persist retention ledger");
    let mut restarted: StatePageRetentionLedger =
        serde_json::from_slice(&encoded).expect("restart retention ledger");
    restarted
        .release_committed_root(&first.target_root)
        .expect("release older root");
    assert!(!restarted.committed_roots.contains_key(&first.target_root));
    assert!(restarted.committed_roots.contains_key(&second.target_root));
    assert!(shared_pages.is_subset(&restarted.reachable_page_ids()));
    assert!(
        shared_pages.is_disjoint(&restarted.sweep_candidates(store.0.borrow().keys().cloned()))
    );
    assert!(restarted.handles.values().all(|handle| {
        handle.target_root != first.target_root
            || handle.phase != StatePageRetentionPhase::Committed
    }));
}

#[test]
fn decision_archive_commit_is_replay_visible_maintenance_ingress() {
    let (mut simulation, _) = Simulation::demo(31).expect("demo should initialize");
    let request_id = DecisionRequestId::new(701);
    simulation
        .enqueue_decision(
            simulation.time(),
            0,
            DecisionIngressRequest::new(
                request_id,
                simulation.revision(),
                DecisionMutation::Cancel {
                    ticket_id: DecisionTicketId::new(999),
                    expected_version: 1,
                    reason: "terminal-missing-ticket".to_owned(),
                },
            ),
        )
        .expect("enqueue deterministic rejection");
    simulation
        .step_canonical()
        .expect("settle decision")
        .expect("decision boundary");
    let mut compact = simulation.into_compacted().expect("compact runtime");
    let key = DecisionHistoryKey::Attempt(request_id);
    let prepared = compact
        .prepare_decision_archive(std::slice::from_ref(&key))
        .expect("prepare terminal attempt");
    let store = DecisionStore::default();
    for blob in &prepared.blobs {
        store
            .store_decision_archive(blob)
            .expect("store decision blob");
    }
    compact
        .commit_decision_archive(&prepared, &store)
        .expect("queue maintenance ingress");
    assert!(matches!(
        compact.decision_history_location(&key),
        DecisionHistoryLocation::Hot
    ));
    compact
        .step_canonical()
        .expect("settle maintenance")
        .expect("maintenance boundary");
    assert!(matches!(
        compact.decision_history_location(&key),
        DecisionHistoryLocation::Archived { .. }
    ));

    let journal = compact
        .replay_journal_with_segments(Vec::new())
        .expect("exact replay journal");
    let replayed =
        Simulation::replay_from_journal(&[], &journal).expect("replay maintenance ingress");
    assert!(matches!(
        replayed.decision_history_location(&key),
        DecisionHistoryLocation::Archived { .. }
    ));
    assert_eq!(
        replayed.checkpoint_hash(),
        compact
            .checkpoint()
            .expect("checkpoint")
            .state
            .checkpoint_hash
    );

    let pages = PageStore::default();
    let checkpoint = compact
        .prepare_paged_checkpoint(None, &pages)
        .expect("prepare sparse decision checkpoint");
    checkpoint
        .store_and_verify(&pages)
        .expect("store sparse decision checkpoint");
    let evidence = compact
        .journal_segment_since(canwu_sim::EvidenceCursor::default())
        .expect("retain checkpoint evidence prefix");
    let restored = Simulation::from_paged_checkpoint_and_journal(
        &checkpoint.checkpoint,
        &pages,
        vec![evidence],
    )
    .expect("root-only decision restore");
    assert!(matches!(
        restored.decision_history_location(&key),
        DecisionHistoryLocation::Unresolved { .. }
    ));
    assert!(matches!(
        restored
            .decision_history_location_with_provider(&key, &pages)
            .expect("load exactly one locator bucket"),
        DecisionHistoryLocation::Archived { .. }
    ));
    let reachability = restored
        .archive_reachability_manifest(&[], &StatePageRetentionLedger::default(), &pages, &())
        .expect("enumerate root-only decision reachability after restart");
    assert!(
        reachability
            .decision_blob_ids
            .contains(&prepared.receipts[0].locator)
    );
    assert!(!reachability.state_page_ids.is_empty());
}

#[test]
fn stale_decision_archive_is_terminally_rejected_without_boundary_rollback() {
    let (mut simulation, _) = Simulation::demo(43).expect("demo should initialize");
    let archived_request = DecisionRequestId::new(801);
    simulation
        .enqueue_decision(
            simulation.time(),
            0,
            DecisionIngressRequest::new(
                archived_request,
                simulation.revision(),
                DecisionMutation::Cancel {
                    ticket_id: DecisionTicketId::new(998),
                    expected_version: 1,
                    reason: "terminal-missing-ticket".to_owned(),
                },
            ),
        )
        .expect("enqueue deterministic rejection");
    simulation
        .step_canonical()
        .expect("settle source attempt")
        .expect("source boundary");
    let mut compact = simulation.into_compacted().expect("compact runtime");
    let key = DecisionHistoryKey::Attempt(archived_request);
    let prepared = compact
        .prepare_decision_archive(std::slice::from_ref(&key))
        .expect("prepare archive");
    let store = DecisionStore::default();
    for blob in &prepared.blobs {
        store.store_decision_archive(blob).expect("store blob");
    }
    compact
        .commit_decision_archive(&prepared, &store)
        .expect("queue verified archive");
    compact
        .enqueue_decision(
            compact.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(802),
                compact.revision(),
                DecisionMutation::Cancel {
                    ticket_id: DecisionTicketId::new(997),
                    expected_version: 1,
                    reason: "make-source-root-stale".to_owned(),
                },
            ),
        )
        .expect("queue earlier-class decision ingress");
    compact
        .step_canonical()
        .expect("stale maintenance must not roll back the boundary")
        .expect("terminal rejection boundary");
    assert!(matches!(
        compact.decision_history_location(&key),
        DecisionHistoryLocation::Hot
    ));
    let boundary = compact.boundaries().last().expect("terminal boundary");
    assert_eq!(boundary.maintenance_changes.len(), 1);
    assert_eq!(
        boundary.maintenance_changes[0].disposition,
        canwu_sim::MaintenanceDisposition::RejectedStale
    );
    assert!(boundary.maintenance_terminal_root.is_some());
}

fn assert_owner_authorized_versions(
    simulation: &Simulation,
    culture_ref: &DomainRecordRef,
    culture_version: &DomainRecordVersionRef,
    law_ref: &DomainRecordRef,
    law_version: &DomainRecordVersionRef,
) {
    assert_eq!(
        simulation
            .current_domain_record_version(culture_ref)
            .expect("current culture version query"),
        Some(culture_version.clone())
    );
    assert_eq!(
        simulation
            .current_domain_record_version(law_ref)
            .expect("current law version query"),
        Some(law_version.clone())
    );
}

fn maintenance_version_from_last_boundary(
    simulation: &Simulation,
    reference: &DomainRecordRef,
) -> DomainRecordVersionRef {
    let boundary = simulation
        .boundaries()
        .last()
        .expect("retirement boundary evidence");
    let (change_index, change) = boundary
        .record_changes
        .iter()
        .enumerate()
        .find(|(_, change)| &change.current.reference == reference)
        .expect("maintenance record change");
    DomainRecordVersionRef {
        record: reference.clone(),
        version: change.current.version,
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: boundary.id,
            change_index: u64::try_from(change_index).expect("bounded change index"),
        },
    }
}

fn owner_authorized_commit_mut<'a>(
    value: &'a mut serde_json::Value,
    request_id: &str,
) -> &'a mut serde_json::Value {
    value["ingress"]
        .as_array_mut()
        .expect("ingress array")
        .iter_mut()
        .find_map(|record| {
            (record["payload"]["type"] == "maintenance"
                && record["payload"]["request"]["maintenance"] == "owner_authorized"
                && record["payload"]["request"]["commit"]["request_id"] == request_id)
                .then_some(&mut record["payload"]["request"]["commit"])
        })
        .expect("owner-authorized commit")
}

fn omit_required_participant_and_rehash(commit: &mut serde_json::Value) -> String {
    commit["participants"]
        .as_array_mut()
        .expect("participant array")
        .retain(|proposal| proposal["plugin"] == "culture-owner");
    let format_version =
        serde_json::from_value::<u32>(commit["format_version"].clone()).expect("format version");
    let request_id =
        serde_json::from_value::<String>(commit["request_id"].clone()).expect("request id");
    let target =
        serde_json::from_value::<OwnerAuthorizedRecordExpectation>(commit["target"].clone())
            .expect("target expectation");
    let requested_at =
        serde_json::from_value::<SimTime>(commit["requested_at"].clone()).expect("requested time");
    let source_root = serde_json::from_value::<String>(commit["source_domain_root"].clone())
        .expect("source root");
    let participants = serde_json::from_value::<Vec<OwnerAuthorizedParticipantProposal>>(
        commit["participants"].clone(),
    )
    .expect("remaining participants");
    let forged_token = canonical_hash(
        "canwu.owner-authorized.maintenance-token.v1",
        &(
            format_version,
            &request_id,
            &target,
            requested_at,
            &source_root,
            &participants,
        ),
    )
    .expect("rehashed forged token");
    commit["token"] = serde_json::json!(&forged_token);
    forged_token
}

#[test]
fn owner_authorized_retirement_is_atomic_and_schema_scoped() {
    let (mut simulation, culture, law, culture_ref, law_ref) = owner_authorized_fixture();
    let draft = owner_authorized_retirement_draft(&simulation, &culture_ref, &law_ref);
    simulation
        .schedule_owner_authorized_maintenance(simulation.time(), 0, draft)
        .expect("enqueue owner-authorized retirement");
    simulation
        .step_canonical()
        .expect("settle retirement")
        .expect("retirement boundary");
    assert!(matches!(
        simulation
            .domain_record(&culture_ref)
            .expect("culture record")
            .lifecycle,
        DomainRecordLifecycle::Retired { .. }
    ));
    let dependency = simulation.domain_record(&law_ref).expect("law dependency");
    assert_eq!(dependency.version, 2);
    assert_eq!(dependency.payload["acknowledged"], serde_json::json!(true));

    let boundary = simulation
        .boundaries()
        .last()
        .expect("retirement boundary evidence");
    assert_eq!(boundary.record_changes.len(), 2);
    let culture_version = maintenance_version_from_last_boundary(&simulation, &culture_ref);
    let law_version = maintenance_version_from_last_boundary(&simulation, &law_ref);
    assert_owner_authorized_versions(
        &simulation,
        &culture_ref,
        &culture_version,
        &law_ref,
        &law_version,
    );

    let restored = Simulation::from_snapshot_with_plugins(simulation.snapshot(), &[&culture, &law])
        .expect("maintenance snapshot restores exact version provenance");
    assert_owner_authorized_versions(
        &restored,
        &culture_ref,
        &culture_version,
        &law_ref,
        &law_version,
    );

    let checkpoint_restored = Simulation::from_checkpoint_journal_with_plugins(
        simulation
            .checkpoint_journal()
            .expect("maintenance checkpoint journal"),
        &[&culture, &law],
    )
    .expect("maintenance checkpoint restores exact version provenance");
    assert_owner_authorized_versions(
        &checkpoint_restored,
        &culture_ref,
        &culture_version,
        &law_ref,
        &law_version,
    );

    let journal = simulation.replay_journal();
    let mut replayed = Simulation::replay_from_journal(&[&culture, &law], &journal)
        .expect("owner-authorized maintenance replays exactly");
    assert_eq!(
        replayed
            .domain_record(&culture_ref)
            .expect("replayed culture record")
            .lifecycle,
        simulation
            .domain_record(&culture_ref)
            .expect("source culture record")
            .lifecycle
    );
    assert_eq!(
        replayed.domain_record(&law_ref),
        simulation.domain_record(&law_ref)
    );
    assert_owner_authorized_versions(
        &replayed,
        &culture_ref,
        &culture_version,
        &law_ref,
        &law_version,
    );
    replayed
        .settle_boundary(BoundaryRequest::at(replayed.time()).with_cadence(SystemCadence::Daily))
        .expect("replayed runtime continues after owner-authorized maintenance");
    assert_owner_authorized_versions(
        &replayed,
        &culture_ref,
        &culture_version,
        &law_ref,
        &law_version,
    );
}

#[test]
fn rehashed_owner_maintenance_cannot_omit_a_required_participant() {
    let (mut simulation, culture, law, culture_ref, law_ref) = owner_authorized_fixture();
    simulation
        .schedule_owner_authorized_maintenance(
            simulation.time(),
            0,
            owner_authorized_retirement_draft(&simulation, &culture_ref, &law_ref),
        )
        .expect("enqueue owner-authorized retirement");
    simulation
        .step_canonical()
        .expect("settle retirement")
        .expect("retirement boundary");

    let mut value = serde_json::to_value(simulation.snapshot()).expect("snapshot value");
    let forged_token = omit_required_participant_and_rehash(owner_authorized_commit_mut(
        &mut value,
        "retire-rights-generation-1",
    ));
    value["boundaries"][0]["maintenance_changes"][0]["token"] = serde_json::json!(forged_token);

    let tampered = serde_json::from_value(value).expect("tampered snapshot shape");
    let Err(error) = Simulation::from_snapshot_with_plugins(tampered, &[&culture, &law]) else {
        panic!("rehashed participant omission must fail closed");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(
        error.message.contains("participant set is incomplete"),
        "{}",
        error.message
    );
}

#[test]
fn rehashed_pending_owner_maintenance_cannot_poison_restored_queue() {
    let (mut simulation, culture, law, culture_ref, law_ref) = owner_authorized_fixture();
    simulation
        .schedule_owner_authorized_maintenance(
            simulation.time(),
            0,
            owner_authorized_retirement_draft(&simulation, &culture_ref, &law_ref),
        )
        .expect("enqueue pending owner-authorized retirement");

    let mut value = serde_json::to_value(simulation.snapshot()).expect("snapshot value");
    omit_required_participant_and_rehash(owner_authorized_commit_mut(
        &mut value,
        "retire-rights-generation-1",
    ));

    let tampered = serde_json::from_value(value).expect("tampered pending snapshot shape");
    let Err(error) = Simulation::from_snapshot_with_plugins(tampered, &[&culture, &law]) else {
        panic!("rehashed pending participant omission must fail closed");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(
        error.message.contains("participant set is incomplete"),
        "{}",
        error.message
    );
}

#[test]
fn rejected_stale_owner_maintenance_is_authorized_and_restores_without_poisoning() {
    let (mut simulation, culture, law, culture_ref, law_ref) = owner_authorized_fixture();
    let first = owner_authorized_retirement_draft(&simulation, &culture_ref, &law_ref);
    let mut stale = owner_authorized_retirement_draft(&simulation, &culture_ref, &law_ref);
    stale.request_id = "retire-rights-generation-1-stale".to_owned();
    simulation
        .schedule_owner_authorized_maintenance(simulation.time(), 1, first)
        .expect("enqueue first owner-authorized retirement");
    simulation
        .schedule_owner_authorized_maintenance(simulation.time(), 0, stale)
        .expect("enqueue later stale owner-authorized retirement");
    simulation
        .step_canonical()
        .expect("settle competing maintenance")
        .expect("maintenance boundary");
    let boundary = simulation
        .boundaries()
        .last()
        .expect("maintenance boundary");
    assert_eq!(boundary.maintenance_changes.len(), 2);
    assert_eq!(
        boundary.maintenance_changes[0].disposition,
        MaintenanceDisposition::Applied
    );
    assert_eq!(
        boundary.maintenance_changes[1].disposition,
        MaintenanceDisposition::RejectedStale
    );

    let snapshot = simulation.snapshot();
    let mut restored = Simulation::from_snapshot_with_plugins(snapshot.clone(), &[&culture, &law])
        .expect("legitimate stale maintenance restores");
    restored
        .settle_boundary(BoundaryRequest::at(restored.time()).with_cadence(SystemCadence::Daily))
        .expect("restored runtime continues after legitimate stale maintenance");

    let mut value = serde_json::to_value(snapshot).expect("snapshot value");
    let forged_token = omit_required_participant_and_rehash(owner_authorized_commit_mut(
        &mut value,
        "retire-rights-generation-1-stale",
    ));
    let stale_terminal = value["boundaries"][0]["maintenance_changes"]
        .as_array_mut()
        .expect("maintenance changes")
        .iter_mut()
        .find(|change| change["disposition"] == "rejected_stale")
        .expect("stale terminal evidence");
    stale_terminal["token"] = serde_json::json!(forged_token);

    let tampered = serde_json::from_value(value).expect("tampered stale snapshot shape");
    let Err(error) = Simulation::from_snapshot_with_plugins(tampered, &[&culture, &law]) else {
        panic!("rehashed stale participant omission must fail closed");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(
        error.message.contains("participant set is incomplete"),
        "{}",
        error.message
    );
}
