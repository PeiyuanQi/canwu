//! Shared world, fiscal content, and application records for the gap fixtures.
//!
//! Two institutions own fiscal scopes: a treasury with several land scopes and
//! a granary with one. An application plugin owns commission records, the
//! authority bases that fiscal bindings cite; renewing a commission advances
//! its version, and a tick makes a boundary event-bearing without touching
//! fiscal state. The same plugin owns one warrant, a record of a kind the
//! fiscal plugin was not configured to read, can change whether a person is
//! alive, and records execution results, the evidence fiscal receipts cite.
//! The fiscal archive tests keep their objects in an in-memory store.

// Each integration target intentionally uses a different subset of these helpers.
#![allow(dead_code)]

use canwu_api::{
    ArchiveProvider, ArchiveStore, ArchiveStoreOutcome, BoundaryContext, BoundaryDirective,
    BoundaryPhase, BoundaryProposal, BoundaryReceipt, BoundarySystemContract, Canwu, CanwuError,
    CommandAttemptOutcome, CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu,
    CustodyState, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordSchema, DomainRecordType,
    DomainRecordVersionRef, DomainRecordVersionSource, DomainValueKindClass, EntityRef,
    EvidenceJournalSegment, Government, GovernmentId, IngressClass, IngressPayload, Issuer,
    KnowledgeHolderRef, KnowledgeRecord, KnowledgeSnapshot, LifeState, MapPoint, OrganizationId,
    PayloadSchema, Person, PersonAvailability, PersonId, PluginIngressDescriptor,
    PluginIngressRequest, PluginRegistrar, ResourceId, Scenario, SimDuration, SimTime,
    SimulationGranularity, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_fiscal::{
    CompiledFiscalCatalog, FiscalAction, FiscalActionBatch, FiscalActionEntry, FiscalActionTarget,
    FiscalAdoptionStage, FiscalAdoptionState, FiscalArchiveError, FiscalArchiveIngressReceiptV1,
    FiscalArchiveRetentionHandleV1, FiscalArchiveRetentionPhaseV1, FiscalArchiveStore,
    FiscalAssessmentBasis, FiscalAuthorityBinding, FiscalCommutationPolicy, FiscalContentPack,
    FiscalContentSelection, FiscalCoverageDeclaration, FiscalCoverageSelector,
    FiscalCoverageStatus, FiscalExecutionEvidence, FiscalExecutionKind,
    FiscalExecutionReceiptPacket, FiscalHistoricalMode, FiscalMechanism, FiscalObserverBinding,
    FiscalPackManifest, FiscalPaymentForm, FiscalPeriodDefinition, FiscalPlugin, FiscalProvenance,
    FiscalReceiptDisposition, FiscalRegionDefinition, FiscalReport, FiscalReportRecord,
    FiscalRuleDefinition, FiscalScopeBinding, FiscalSettledAction, FiscalState,
    FiscalTransitionDefinition, HistoricalConfidence, HistoricalYearWindow, audit_fiscal_records,
    compile_fiscal_content, enqueue_fiscal_archive, finalize_fiscal_archive_retention,
    fiscal_action_command, fiscal_action_target, fiscal_reports, fiscal_target_revision,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

pub const PRINCIPAL: PersonId = PersonId::new(1);
pub const DEPUTY: PersonId = PersonId::new(2);
pub const SUCCESSOR: PersonId = PersonId::new(3);
pub const AUDITOR: PersonId = PersonId::new(4);
pub const GRANARY_PRINCIPAL: PersonId = PersonId::new(5);
pub const SECOND_AUDITOR: PersonId = PersonId::new(6);

pub const TREASURY_AUTHORITY: &str = "authority.treasury";
pub const GRANARY_AUTHORITY: &str = "authority.granary";
pub const GRANARY_SCOPE: &str = "scope.granary.0";
/// A treasury scope whose rule no adoption has made operational yet.
pub const PENDING_SCOPE: &str = "scope.treasury.pending";
pub const TREASURY_SCOPES: usize = 4;
pub const RULE: &str = "rule.land";
/// The rule the fixture's one transition implements in place of [`RULE`].
pub const REFORMED_RULE: &str = "rule.land.reformed";
/// The fixture's one transition, eligible from year 160.
pub const REFORM: &str = "reform.land";
pub const OBSERVER: &str = "observer.principal";

pub const DEPUTY_COMMISSION: &str = "commission.deputy";
pub const SUCCESSOR_COMMISSION: &str = "commission.successor";
pub const AUDITOR_COMMISSION: &str = "commission.auditor";

const COMMISSION_PLUGIN: &str = "fixture-commissions";
const RENEW_COMMISSION_INGRESS: &str = "renew_commission_v1";
const TICK_INGRESS: &str = "tick_v1";
const LIFE_INGRESS: &str = "set_life_v1";
const EVIDENCE_INGRESS: &str = "record_evidence_v1";

#[must_use]
pub fn treasury() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(1))
}

#[must_use]
pub fn granary() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(2))
}

#[must_use]
pub fn treasury_scope(index: usize) -> String {
    format!("scope.treasury.{index}")
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommissionPayload {
    term: u32,
}

/// An application-owned record of a kind the fiscal plugin reads as an
/// authority basis.
pub struct Commission;

impl DomainRecordType for Commission {
    type Payload = CommissionPayload;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.authority";
    const NAME: &'static str = "commission";
}

fn commission_reference(id: &str) -> TypedDomainRecordRef<Commission> {
    TypedDomainRecordRef::new(id)
}

fn commission_state_key() -> StateKey {
    DomainRecordSchema::for_record::<Commission>().state_key()
}

/// The version of a commission that the scenario installs.
#[must_use]
pub fn initial_commission(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: commission_reference(id).into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

/// An application-owned record of one external execution result, which a
/// fiscal receipt cites as its evidence.
pub struct ExecutionResult;

impl DomainRecordType for ExecutionResult {
    type Payload = FiscalExecutionEvidence;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.execution";
    const NAME: &'static str = "result";
}

/// An application-owned record of a kind the fiscal plugin reads as
/// execution evidence but the base state does not approve.
pub struct UnapprovedResult;

impl DomainRecordType for UnapprovedResult {
    type Payload = FiscalExecutionEvidence;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.execution";
    const NAME: &'static str = "unapproved";
}

fn execution_state_key() -> StateKey {
    DomainRecordSchema::for_record::<ExecutionResult>().state_key()
}

fn unapproved_state_key() -> StateKey {
    DomainRecordSchema::for_record::<UnapprovedResult>().state_key()
}

/// An application-owned record of a kind the fiscal plugin does not read.
pub struct Warrant;

impl DomainRecordType for Warrant {
    type Payload = CommissionPayload;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.authority";
    const NAME: &'static str = "warrant";
}

fn warrant_reference() -> TypedDomainRecordRef<Warrant> {
    TypedDomainRecordRef::new("warrant.unread")
}

/// The current version of the one warrant the scenario installs.
#[must_use]
pub fn initial_warrant() -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: warrant_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

struct CommissionPlugin;

impl SimulationPlugin for CommissionPlugin {
    fn name(&self) -> &'static str {
        COMMISSION_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c2"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::for_record::<Commission>())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<Warrant>())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<ExecutionResult>())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<UnapprovedResult>())?;
        for (name, description) in [
            (
                RENEW_COMMISSION_INGRESS,
                "Renew one commission as a new record version",
            ),
            (
                TICK_INGRESS,
                "Make one boundary event-bearing without changing any record",
            ),
            (LIFE_INGRESS, "Set whether one person is alive"),
            (EVIDENCE_INGRESS, "Record the listed execution results"),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class: IngressClass::Information,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        let mut renew = BoundarySystemContract::new(
            "renew-commissions-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        renew.reads = vec![
            StateKey::core_ingress(),
            commission_state_key(),
            execution_state_key(),
            unapproved_state_key(),
        ];
        renew.reads.sort();
        renew.writes = vec![
            StateKey::core_person_availability(),
            commission_state_key(),
            execution_state_key(),
            unapproved_state_key(),
        ];
        renew.writes.sort();
        renew.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(renew, renew_commissions)
    }
}

fn renew_commissions(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } = &ingress.payload
            && plugin == COMMISSION_PLUGIN
            && packet_type == EVIDENCE_INGRESS
        {
            for (unapproved, result) in
                serde_json::from_value::<Vec<(bool, FiscalExecutionEvidence)>>(payload.clone())
                    .expect("execution results")
            {
                let record = if unapproved {
                    DomainRecordDraft::from_typed(
                        TypedDomainRecordRef::<UnapprovedResult>::new(&result.id),
                        &result,
                    )?
                } else {
                    DomainRecordDraft::from_typed(execution_reference(&result.id), &result)?
                };
                // A result recorded again under its ID is its record's next
                // version, so a long run keeps a bounded number of records.
                let mutation = match view.domain_record(&record.reference)? {
                    Some(existing) => DomainRecordMutation::Update {
                        expected_version: existing.version,
                        record,
                    },
                    None => DomainRecordMutation::Create { record },
                };
                directives.push(BoundaryDirective::MutateRecord {
                    mutation,
                    summary: "Record one execution result".to_owned(),
                });
            }
        } else if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } = &ingress.payload
            && plugin == COMMISSION_PLUGIN
            && packet_type == LIFE_INGRESS
        {
            let life = if payload["alive"].as_bool().expect("alive") {
                LifeState::Alive
            } else {
                LifeState::Dead
            };
            directives.push(BoundaryDirective::SetPersonAvailability {
                person: PersonId::new(payload["person"].as_u64().expect("person")),
                availability: PersonAvailability::new(life, CustodyState::Free, view.time()),
                summary: "Set whether one person is alive".to_owned(),
            });
        } else if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } = &ingress.payload
            && plugin == COMMISSION_PLUGIN
            && packet_type == RENEW_COMMISSION_INGRESS
        {
            let reference = commission_reference(payload["id"].as_str().expect("commission id"));
            let record = view
                .typed_domain_record(&reference)?
                .expect("commission record");
            let mut commission = record.decode_payload::<Commission>()?;
            commission.term += 1;
            directives.push(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: DomainRecordDraft::from_typed(reference, &commission)?,
                    expected_version: record.version,
                },
                summary: "Renew one commission".to_owned(),
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The plugins every fixture run registers.
pub struct Fixture {
    commissions: CommissionPlugin,
    fiscal: FiscalPlugin,
}

impl Fixture {
    #[must_use]
    pub fn new() -> Self {
        Self {
            commissions: CommissionPlugin,
            fiscal: FiscalPlugin::new([
                DomainRecordKind::for_type::<ExecutionResult>(),
                DomainRecordKind::for_type::<UnapprovedResult>(),
            ])
            .with_authority_basis_kinds([DomainRecordKind::for_type::<Commission>()]),
        }
    }

    #[must_use]
    pub fn plugins(&self) -> [&dyn SimulationPlugin; 2] {
        [&self.commissions, &self.fiscal]
    }

    /// The fixture's plugins and one more.
    #[must_use]
    pub fn plugins_with<'a>(
        &'a self,
        extra: &'a dyn SimulationPlugin,
    ) -> [&'a dyn SimulationPlugin; 3] {
        [&self.commissions, &self.fiscal, extra]
    }

    pub fn try_runtime(&self, scenario: Scenario) -> Result<Canwu, CanwuError> {
        Canwu::new_with_plugins(41, scenario, &self.plugins())
    }

    #[must_use]
    pub fn runtime(&self, scenario: Scenario) -> Canwu {
        self.try_runtime(scenario).expect("fixture runtime")
    }

    /// Asserts that a snapshot restore and an exact replay both reproduce the
    /// run, fiscal state and command attempts included.
    pub fn assert_restore_and_replay(&self, canwu: &Canwu) {
        let plugins = self.plugins();
        let snapshot = canwu.snapshot_json().expect("snapshot");
        let restored =
            Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
        assert_eq!(fiscal_state(&restored), fiscal_state(canwu));
        assert_eq!(
            restored.snapshot_json().expect("restored snapshot"),
            snapshot
        );
        let replayed =
            Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replayed run");
        assert_eq!(replayed.command_attempts(), canwu.command_attempts());
        assert_eq!(
            replayed.snapshot_json().expect("replayed snapshot"),
            snapshot
        );
    }
}

#[must_use]
pub fn catalog() -> CompiledFiscalCatalog {
    let window = HistoricalYearWindow {
        start: 100,
        end: 200,
    };
    let provenance = BTreeSet::from(["source.fixture".to_owned()]);
    let pack = FiscalContentPack {
        manifest: FiscalPackManifest {
            pack_id: "fixture.gaps".to_owned(),
            pack_version: "1.0.0".to_owned(),
            schema_version: 1,
            license: "Apache-2.0".to_owned(),
            title: "Gap fixture".to_owned(),
            historical_scope: window.clone(),
            period_ids: vec!["period.a".to_owned()],
            region_ids: vec!["region.a".to_owned()],
            mechanisms: vec![FiscalMechanism::LandTax],
        },
        periods: vec![FiscalPeriodDefinition {
            id: "period.a".to_owned(),
            label: "Fixture period".to_owned(),
            window: window.clone(),
            branch: None,
        }],
        regions: vec![FiscalRegionDefinition {
            id: "region.a".to_owned(),
            label: "Fixture region".to_owned(),
        }],
        institutions: Vec::new(),
        rules: [(RULE, "Land levy"), (REFORMED_RULE, "Reformed land levy")]
            .into_iter()
            .map(|(id, label)| FiscalRuleDefinition {
                id: id.to_owned(),
                revision: 1,
                label: label.to_owned(),
                mechanism: FiscalMechanism::LandTax,
                legal_window: window.clone(),
                jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
                subject_scope: "registered_land".to_owned(),
                assessment_basis: FiscalAssessmentBasis::RegisteredLand,
                payment_forms: BTreeSet::from([FiscalPaymentForm::Grain]),
                commutation: FiscalCommutationPolicy::Disabled,
                earmark_ids: BTreeSet::new(),
                provenance_ids: provenance.clone(),
                confidence: HistoricalConfidence::Medium,
            })
            .collect(),
        // Eligible only from year 160, so a run at the fixture's year 150 has
        // no transition candidate until its historical context moves on.
        transitions: vec![FiscalTransitionDefinition {
            id: REFORM.to_owned(),
            label: "Land reform".to_owned(),
            from_rule_ids: BTreeSet::from([RULE.to_owned()]),
            to_rule_ids: BTreeSet::from([REFORMED_RULE.to_owned()]),
            observed_window: HistoricalYearWindow {
                start: 180,
                end: 200,
            },
            eligibility_window: HistoricalYearWindow {
                start: 160,
                end: 200,
            },
            jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
            supersedes_or_suspends: BTreeSet::from([RULE.to_owned()]),
            prerequisite_ids: BTreeSet::new(),
            provenance_ids: provenance.clone(),
            confidence: HistoricalConfidence::Medium,
        }],
        coverage: vec![FiscalCoverageDeclaration {
            id: "coverage.a".to_owned(),
            priority: 1,
            selector: FiscalCoverageSelector {
                period_ids: BTreeSet::new(),
                region_ids: BTreeSet::new(),
                mechanisms: BTreeSet::new(),
            },
            status: FiscalCoverageStatus::Supported,
            definition_ids: BTreeSet::from([
                RULE.to_owned(),
                REFORMED_RULE.to_owned(),
                REFORM.to_owned(),
            ]),
            provenance_ids: provenance,
        }],
        provenance: vec![FiscalProvenance {
            id: "source.fixture".to_owned(),
            citation: "Synthetic fixture".to_owned(),
            url: "https://example.invalid/fixture".to_owned(),
            claim_scope: "test content only".to_owned(),
            confidence: HistoricalConfidence::Medium,
            forbidden_inferences: vec!["no historical claim".to_owned()],
        }],
    };
    compile_fiscal_content(
        &pack,
        FiscalContentSelection {
            historical_year: 150,
            ..FiscalContentSelection::default()
        },
    )
    .expect("fixture fiscal catalog")
}

fn scope(id: &str, institution: EntityRef) -> FiscalScopeBinding {
    FiscalScopeBinding {
        id: id.to_owned(),
        institution,
        jurisdiction_id: "region.a".to_owned(),
        subject_scope: "registered_land".to_owned(),
        mechanism: FiscalMechanism::LandTax,
        authoritative_granularity: SimulationGranularity::Aggregate,
    }
}

fn principal_binding(id: &str, institution: EntityRef, actor: PersonId) -> FiscalAuthorityBinding {
    FiscalAuthorityBinding {
        id: id.to_owned(),
        institution,
        authorized_actor: Some(actor),
        acting_actor: None,
        authority_basis: None,
        rebound_by: None,
    }
}

/// Bindings, scopes, and adoptions of both institutions, with no assessment
/// and no derived field computed.
#[must_use]
pub fn base_state() -> FiscalState {
    let mut state = FiscalState::new(150, FiscalHistoricalMode::Counterfactual, SimTime::EPOCH);
    for binding in [
        principal_binding(TREASURY_AUTHORITY, treasury(), PRINCIPAL),
        principal_binding(GRANARY_AUTHORITY, granary(), GRANARY_PRINCIPAL),
    ] {
        state.add_authority_binding(binding);
    }
    let mut operational_scopes: Vec<_> = (0..TREASURY_SCOPES)
        .map(|index| scope(&treasury_scope(index), treasury()))
        .collect();
    operational_scopes.push(scope(GRANARY_SCOPE, granary()));
    for binding in operational_scopes {
        let adoption_id = binding.id.replacen("scope", "adopt", 1);
        let scope_binding_id = binding.id.clone();
        state.add_scope(binding);
        state
            .add_adoption(FiscalAdoptionState {
                id: adoption_id,
                rule_id: RULE.to_owned(),
                scope_binding_id,
                stage: FiscalAdoptionStage::Implemented,
                generation: 1,
                changed_at: SimTime::EPOCH,
                source_action_id: None,
            })
            .expect("adoption of a known scope");
    }
    state.add_scope(scope(PENDING_SCOPE, treasury()));
    state
        .root
        .execution_evidence_kinds
        .insert(DomainRecordKind::for_type::<ExecutionResult>());
    state.add_observer(observer_binding(
        OBSERVER,
        PRINCIPAL,
        BTreeSet::from([treasury()]),
    ));
    state
}

/// An observer binding of `actor`'s person holder, which no scenario action
/// may change.
#[must_use]
pub fn observer_binding(
    id: &str,
    actor: PersonId,
    visible_institutions: BTreeSet<EntityRef>,
) -> FiscalObserverBinding {
    FiscalObserverBinding {
        id: id.to_owned(),
        actor: Some(actor),
        knowledge_holder: KnowledgeHolderRef::Person(actor),
        visible_institutions,
        confidence_per_mille: 1_000,
        controlled_by: None,
        rebound_by: None,
        removed: None,
    }
}

/// The world, the commission records, and the records the given fiscal state
/// partitions into.
#[must_use]
pub fn scenario_from_state(state: FiscalState) -> Scenario {
    let catalog = catalog();
    let records = state.into_records(&catalog).expect("fiscal records");
    scenario_from_records(catalog.into_record().expect("catalog record"), records)
}

/// The world and the commission records around already encoded fiscal records.
#[must_use]
pub fn scenario_from_records(catalog: DomainRecord, fiscal: Vec<DomainRecord>) -> Scenario {
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    let world = WorldSnapshot {
        people: [
            PRINCIPAL,
            DEPUTY,
            SUCCESSOR,
            AUDITOR,
            GRANARY_PRINCIPAL,
            SECOND_AUDITOR,
        ]
        .into_iter()
        .map(|id| Person {
            id,
            name: format!("Person {id}"),
            government,
            current_location: territory,
            roles: Vec::new(),
            transit: None,
        })
        .collect(),
        governments: vec![Government {
            id: government,
            name: "Fixture authority".to_owned(),
            capital: territory,
        }],
        territories: vec![Territory {
            id: territory,
            name: "Fixture district".to_owned(),
            controller: government,
            position: MapPoint::default(),
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    };
    let mut entities = world.entities();
    entities.extend([treasury(), granary()]);
    let payload = CommissionPayload { term: 1 };
    let mut domain_records: Vec<_> = [DEPUTY_COMMISSION, SUCCESSOR_COMMISSION, AUDITOR_COMMISSION]
        .into_iter()
        .map(|id| DomainRecordDraft::from_typed(commission_reference(id), &payload))
        .chain([DomainRecordDraft::from_typed(warrant_reference(), &payload)])
        .map(|draft| {
            let draft = draft.expect("application record draft");
            DomainRecord {
                reference: draft.reference,
                owner: COMMISSION_PLUGIN.to_owned(),
                class: DomainRecordClass::Record,
                version: 1,
                lifecycle: DomainRecordLifecycle::Active,
                payload: draft.payload,
                references: draft.references,
            }
        })
        .collect();
    domain_records.push(catalog);
    domain_records.extend(fiscal);
    Scenario {
        start_time: SimTime::EPOCH,
        entities,
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records,
    }
}

/// A scenario whose fiscal state is the base state after `configure`; its
/// derived fields are computed when it is partitioned.
#[must_use]
pub fn scenario(configure: impl FnOnce(&mut FiscalState)) -> Scenario {
    let mut state = base_state();
    configure(&mut state);
    scenario_from_state(state)
}

/// The run's fiscal state, assembled from its records after checking that
/// they are exactly the partition of a valid state.
#[must_use]
pub fn fiscal_state(canwu: &Canwu) -> FiscalState {
    audit_fiscal_records(canwu).expect("fiscal records audit")
}

/// The version of every fiscal record of the run, by record: what a
/// boundary that writes no fiscal record leaves unchanged.
#[must_use]
pub fn fiscal_record_versions(canwu: &Canwu) -> BTreeMap<String, u64> {
    canwu
        .domain_records()
        .filter(|record| record.reference.kind.namespace == "canwu.fiscal")
        .map(|record| (record.reference.to_string(), record.version))
        .collect()
}

/// The revision of one action target now.
#[must_use]
pub fn target_revision<R: canwu_api::CanwuRun + ?Sized>(
    runtime: &R,
    target: &FiscalActionTarget,
) -> u64 {
    fiscal_target_revision(runtime, target)
        .expect("target revision read")
        .expect("an existing target")
}

/// The revision of a scope's ledger now.
#[must_use]
pub fn ledger_revision(canwu: &Canwu, scope: &str) -> u64 {
    target_revision(canwu, &FiscalActionTarget::Ledger(scope.to_owned()))
}

/// The one fiscal report `actor` holds, assembled from its parts.
#[must_use]
pub fn fiscal_report(canwu: &Canwu, actor: PersonId) -> FiscalReport {
    let viewer = canwu.viewer_for_actor(actor).expect("observer viewer");
    let mut reports = fiscal_reports(&viewer).expect("observer reports");
    assert_eq!(reports.len(), 1, "one report for the actor's one binding");
    reports.pop_first().expect("the actor's report").1
}

/// Every fiscal report record the run has published to `actor`, in the order
/// published, as the retained boundary evidence holds them.
#[must_use]
pub fn published_report_records(
    canwu: &Canwu,
    actor: PersonId,
) -> Vec<(KnowledgeRecord, FiscalReportRecord)> {
    let holder = KnowledgeHolderRef::Person(actor);
    canwu
        .boundaries()
        .iter()
        .flat_map(|boundary| &boundary.knowledge_changes)
        .flat_map(|change| change.records_of(&holder).collect::<Vec<_>>())
        .map(|record| {
            let payload = serde_json::from_value(record.payload.clone())
                .expect("a fiscal report record payload");
            (record, payload)
        })
        .collect()
}

/// One fiscal action that has not been stamped with its target's revision
/// yet.
#[derive(Clone)]
pub struct Draft {
    pub action_id: String,
    pub binding: String,
    pub action: FiscalAction,
}

impl Draft {
    /// The one-entry batch an issuer who decided against `revision` of the
    /// action's target sends.
    #[must_use]
    pub fn against(self, revision: u64) -> FiscalActionBatch {
        let binding = self.binding.clone();
        FiscalActionBatch::single(binding, self.entry(revision))
    }

    /// The entry an issuer who decided against `revision` sends.
    #[must_use]
    pub fn entry(self, revision: u64) -> FiscalActionEntry {
        FiscalActionEntry {
            action_id: self.action_id,
            expected_revision: revision,
            action: self.action,
        }
    }

    /// The action's target, as admission resolves it on `runtime` now.
    #[must_use]
    pub fn target<R: canwu_api::CanwuRun + ?Sized>(&self, runtime: &R) -> FiscalActionTarget {
        fiscal_action_target(runtime, &self.binding, &self.action).expect("the draft's target")
    }

    /// The one-entry batch stamped with its target's revision on `runtime`
    /// now. An action whose target admission cannot resolve, which admission
    /// will refuse, is stamped with revision 1.
    #[must_use]
    pub fn current<R: canwu_api::CanwuRun + ?Sized>(self, runtime: &R) -> FiscalActionBatch {
        let revision = fiscal_action_target(runtime, &self.binding, &self.action)
            .ok()
            .and_then(|target| {
                fiscal_target_revision(runtime, &target).expect("target revision read")
            })
            .unwrap_or(1);
        self.against(revision)
    }
}

#[must_use]
pub fn draft(action_id: &str, binding: &str, action: FiscalAction) -> Draft {
    Draft {
        action_id: action_id.to_owned(),
        binding: binding.to_owned(),
        action,
    }
}

/// An assessment of `scope` for `cycle`, issued under `binding`.
#[must_use]
pub fn assessment(action_id: &str, binding: &str, scope: &str, cycle: &str) -> Draft {
    draft(
        action_id,
        binding,
        FiscalAction::OpenAssessment {
            assessment_id: format!("{action_id}.assessment"),
            rule_id: RULE.to_owned(),
            scope_binding_id: scope.to_owned(),
            accounting_cycle_id: cycle.to_owned(),
            quantity: 100,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
        },
    )
}

/// Enqueues the batches as tracked commands due now, in order, so that one
/// boundary admits all of them and the next settles their actions.
pub fn enqueue_requests(canwu: &mut Canwu, commands: &[(PersonId, FiscalActionBatch)]) {
    let revision = canwu.revision();
    for (offset, (issuer, request)) in (0_u64..).zip(commands) {
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                CommandRequest::new(
                    CommandRequestId::new(revision + offset + 1),
                    revision + offset,
                    CommandEnvelope::new(
                        Issuer::Actor(*issuer),
                        fiscal_action_command(request).expect("fiscal command"),
                    )
                    .at_time(canwu.time()),
                ),
            )
            .expect("tracked fiscal command");
    }
}

/// Enqueues the drafts, each a one-entry batch against the revision its
/// target has now, as issuers who all read the same records would send them.
pub fn enqueue(canwu: &mut Canwu, commands: Vec<(PersonId, Draft)>) {
    let commands: Vec<_> = commands
        .into_iter()
        .map(|(issuer, draft)| (issuer, draft.current(canwu)))
        .collect();
    enqueue_requests(canwu, &commands);
}

/// Enqueues the drafts as batches: consecutive drafts of one issuer under
/// one binding go into one batch command, in order, each entry against the
/// revision its target has now.
pub fn enqueue_batches(canwu: &mut Canwu, commands: Vec<(PersonId, Draft)>) {
    let mut batches: Vec<(PersonId, FiscalActionBatch)> = Vec::new();
    for (issuer, draft) in commands {
        let revision = fiscal_action_target(canwu, &draft.binding, &draft.action)
            .ok()
            .and_then(|target| {
                fiscal_target_revision(canwu, &target).expect("target revision read")
            })
            .unwrap_or(1);
        match batches.last_mut() {
            Some((last, batch))
                if *last == issuer && batch.authority_binding_id == draft.binding =>
            {
                batch.entries.push(draft.entry(revision));
            }
            _ => batches.push((issuer, draft.against(revision))),
        }
    }
    enqueue_requests(canwu, &batches);
}

/// Settles everything due and returns the receipts of the boundaries it took.
pub fn settle(canwu: &mut Canwu) -> Vec<BoundaryReceipt> {
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("fixture boundaries")
}

/// Admits and settles the drafts, each from its own issuer.
pub fn submit(canwu: &mut Canwu, commands: Vec<(PersonId, Draft)>) -> Vec<BoundaryReceipt> {
    enqueue(canwu, commands);
    settle(canwu)
}

#[must_use]
pub fn sealed_fiscal_state(sealed: &CompactedCanwu) -> FiscalState {
    canwu_fiscal::load_fiscal_state(sealed)
        .expect("fiscal state")
        .expect("a fiscal state")
}

/// Admits and settles the drafts on a compacted run, as `submit` does on a
/// full one.
pub fn submit_sealed(sealed: &mut CompactedCanwu, commands: Vec<(PersonId, Draft)>) {
    let batches: Vec<_> = commands
        .into_iter()
        .map(|(issuer, draft)| (issuer, draft.current(&*sealed)))
        .collect();
    let revision = sealed.revision();
    for (offset, (issuer, batch)) in (0_u64..).zip(batches) {
        let command = fiscal_action_command(&batch).expect("fiscal command");
        sealed
            .enqueue_command(
                sealed.time(),
                0,
                CommandRequest::new(
                    CommandRequestId::new(revision + offset + 1),
                    revision + offset,
                    CommandEnvelope::new(Issuer::Actor(issuer), command).at_time(sealed.time()),
                ),
            )
            .expect("tracked fiscal command");
    }
    sealed
        .advance_canonical(SimDuration::minutes(1))
        .expect("sealed boundaries");
}

/// The outcome of `action_id`, as its settled event records it: the last one
/// the run's retained events hold.
#[must_use]
pub fn outcome(canwu: &Canwu, action_id: &str) -> Option<FiscalSettledAction> {
    FiscalSettledAction::from_events(canwu.events())
        .into_iter()
        .rev()
        .find(|settled| settled.action_id == action_id)
}

/// When the settled event of `action_id` was recorded.
pub fn settled_at(canwu: &Canwu, action_id: &str) -> SimTime {
    canwu
        .events()
        .iter()
        .find(|event| {
            event.kind.plugin_identity()
                == Some((
                    canwu_fiscal::PLUGIN_NAME,
                    canwu_fiscal::FISCAL_ACTION_SETTLED_EVENT,
                ))
                && FiscalSettledAction::from_summary(&event.summary)
                    .is_some_and(|settled| settled.action_id == action_id)
        })
        .map_or_else(
            || panic!("{action_id} has no settled event"),
            |event| event.timestamp,
        )
}

/// The error of the command attempt that carried `action_id`, when admission
/// rejected it.
#[must_use]
pub fn admission_rejection(canwu: &Canwu, action_id: &str) -> Option<CanwuError> {
    canwu.command_attempts().iter().rev().find_map(|attempt| {
        let canwu_api::Command::Plugin { payload, .. } = &attempt.envelope.command else {
            return None;
        };
        if !payload["entries"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["action_id"] == action_id))
        {
            return None;
        }
        match &attempt.outcome {
            CommandAttemptOutcome::Rejected { error } => Some(error.clone()),
            CommandAttemptOutcome::Accepted { .. } => None,
        }
    })
}

/// Ingress that renews one commission at `at`.
#[must_use]
pub fn renewal_at(at: SimTime, commission: &str) -> PluginIngressRequest {
    PluginIngressRequest::new(
        COMMISSION_PLUGIN,
        RENEW_COMMISSION_INGRESS,
        at,
        serde_json::json!({ "id": commission }),
    )
}

/// Ingress at `at` that belongs to another plugin and changes nothing.
#[must_use]
pub fn tick_at(at: SimTime) -> PluginIngressRequest {
    PluginIngressRequest::new(COMMISSION_PLUGIN, TICK_INGRESS, at, serde_json::json!({}))
}

/// Queues a renewal of one commission for the next boundary.
pub fn enqueue_renewal(canwu: &mut Canwu, commission: &str) {
    canwu
        .enqueue_plugin_ingress(renewal_at(canwu.time(), commission))
        .expect("commission renewal ingress");
}

/// Ingress at `at` that sets whether `person` is alive.
#[must_use]
pub fn life_at(at: SimTime, person: PersonId, alive: bool) -> PluginIngressRequest {
    PluginIngressRequest::new(
        COMMISSION_PLUGIN,
        LIFE_INGRESS,
        at,
        serde_json::json!({ "person": person.get(), "alive": alive }),
    )
}

/// Queues, for the next boundary, a change of whether `person` is alive.
pub fn enqueue_life(canwu: &mut Canwu, person: PersonId, alive: bool) {
    canwu
        .enqueue_plugin_ingress(life_at(canwu.time(), person, alive))
        .expect("life ingress");
}

/// Queues ingress that belongs to another plugin and changes nothing.
pub fn enqueue_tick(canwu: &mut Canwu) {
    canwu
        .enqueue_plugin_ingress(tick_at(canwu.time()))
        .expect("tick ingress");
}

#[must_use]
pub fn current_commission(canwu: &Canwu, commission: &str) -> DomainRecordVersionRef {
    canwu
        .current_domain_record_version(&commission_reference(commission).into_untyped())
        .expect("current commission version query")
        .expect("current commission version")
}

fn execution_reference(id: &str) -> TypedDomainRecordRef<ExecutionResult> {
    TypedDomainRecordRef::new(id)
}

/// What one execution of `quantity` against `request` of `scope` claims:
/// a fulfilled collection from the principal to the treasury, under its own
/// external operation.
#[must_use]
pub fn execution_claim(
    id: &str,
    scope: &str,
    request: &str,
    quantity: u64,
) -> FiscalExecutionEvidence {
    FiscalExecutionEvidence {
        id: id.to_owned(),
        external_operation_id: format!("operation.{id}"),
        scope_binding_id: scope.to_owned(),
        request_id: request.to_owned(),
        quantity,
        unit: "grain".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(PRINCIPAL),
        target: treasury(),
        disposition: FiscalReceiptDisposition::Fulfilled,
    }
}

/// An authorization of a collection of `quantity` from the principal to the
/// treasury, against `assessment` of `scope`.
#[must_use]
pub fn collection(
    action_id: &str,
    scope: &str,
    assessment: &str,
    request: &str,
    quantity: u64,
) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::AuthorizeExecution {
            request_id: request.to_owned(),
            scope_binding_id: scope.to_owned(),
            assessment_id: assessment.to_owned(),
            kind: FiscalExecutionKind::Collect,
            quantity,
            unit: "grain".to_owned(),
            resource: ResourceId::new(1),
            source: EntityRef::Person(PRINCIPAL),
            target: treasury(),
            purpose: "fixture collection".to_owned(),
        },
    )
}

/// A close of one accounting cycle book of `scope`, issued by the treasury.
#[must_use]
pub fn close(action_id: &str, scope: &str, cycle: &str) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::CloseAccountingCycle {
            scope_binding_id: scope.to_owned(),
            accounting_cycle_id: cycle.to_owned(),
        },
    )
}

/// Records the execution results in one boundary and returns the exact
/// version of each, in order. A result marked `true` is of the kind the
/// state does not approve.
pub fn record_results(
    canwu: &mut Canwu,
    results: &[(bool, FiscalExecutionEvidence)],
) -> Vec<DomainRecordVersionRef> {
    enqueue_results(canwu, results);
    settle(canwu);
    result_versions(canwu, results)
}

/// Queues the recording of the execution results for the next boundary.
pub fn enqueue_results(canwu: &mut Canwu, results: &[(bool, FiscalExecutionEvidence)]) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            COMMISSION_PLUGIN,
            EVIDENCE_INGRESS,
            canwu.time(),
            serde_json::to_value(results).expect("results payload"),
        ))
        .expect("evidence ingress");
}

/// The current version of each recorded execution result, in order.
#[must_use]
pub fn result_versions(
    canwu: &Canwu,
    results: &[(bool, FiscalExecutionEvidence)],
) -> Vec<DomainRecordVersionRef> {
    results
        .iter()
        .map(|(unapproved, result)| {
            let reference = if *unapproved {
                TypedDomainRecordRef::<UnapprovedResult>::new(&result.id).into_untyped()
            } else {
                execution_reference(&result.id).into_untyped()
            };
            canwu
                .current_domain_record_version(&reference)
                .expect("evidence version query")
                .expect("evidence version")
        })
        .collect()
}

/// A receipt of `scope` citing `evidence`.
#[must_use]
pub fn receipt_packet(
    id: &str,
    scope: &str,
    request: &str,
    evidence: impl IntoIterator<Item = DomainRecordVersionRef>,
) -> FiscalExecutionReceiptPacket {
    FiscalExecutionReceiptPacket {
        receipt_id: id.to_owned(),
        scope_binding_id: scope.to_owned(),
        request_id: request.to_owned(),
        external_evidence: evidence.into_iter().collect(),
    }
}

/// Every refusal event the run's retained events hold, by summary.
#[must_use]
pub fn refusals(canwu: &Canwu) -> Vec<String> {
    canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity()
                == Some((
                    canwu_fiscal::PLUGIN_NAME,
                    canwu_fiscal::FISCAL_INGRESS_REJECTED_EVENT,
                ))
        })
        .map(|event| event.summary.clone())
        .collect()
}

/// A fiscal archive store in memory: every object by namespace and ID, and
/// the latest phase of every retention handle.
#[derive(Default)]
pub struct MemoryArchiveStore {
    objects: RwLock<BTreeMap<(String, String), Vec<u8>>>,
    handles: RwLock<BTreeMap<String, FiscalArchiveRetentionPhaseV1>>,
}

impl MemoryArchiveStore {
    /// How many objects of `namespace` the store holds.
    #[must_use]
    pub fn count(&self, namespace: &str) -> usize {
        self.objects
            .read()
            .unwrap()
            .keys()
            .filter(|(stored, _)| stored == namespace)
            .count()
    }

    /// The phase the store last recorded for one retention handle.
    #[must_use]
    pub fn handle_phase(&self, handle_id: &str) -> Option<FiscalArchiveRetentionPhaseV1> {
        self.handles.read().unwrap().get(handle_id).copied()
    }
}

impl FiscalArchiveStore for MemoryArchiveStore {
    fn store_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), FiscalArchiveError> {
        self.objects
            .write()
            .unwrap()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, FiscalArchiveError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_fiscal_archive_retention(
        &self,
        handle: &FiscalArchiveRetentionHandleV1,
    ) -> Result<(), FiscalArchiveError> {
        self.handles
            .write()
            .unwrap()
            .insert(handle.handle_id.clone(), handle.phase);
        Ok(())
    }

    fn finalize_fiscal_archive_retention(
        &self,
        handle_id: &str,
        phase: FiscalArchiveRetentionPhaseV1,
    ) -> Result<(), FiscalArchiveError> {
        self.handles
            .write()
            .unwrap()
            .insert(handle_id.to_owned(), phase);
        Ok(())
    }
}

/// Prepares one archive commit of up to `books` closed books from the run's
/// state and `store`, and queues it. The run has never sealed its evidence.
pub fn enqueue_archive(
    canwu: &mut Canwu,
    store: &MemoryArchiveStore,
    books: usize,
) -> FiscalArchiveIngressReceiptV1 {
    let prepared = fiscal_state(canwu)
        .prepare_fiscal_archive(books, store, None)
        .expect("archive batch");
    enqueue_fiscal_archive(canwu, &prepared, store, None).expect("archive commit queued")
}

/// Archives up to `books` closed books: prepares, stores, verifies, and
/// queues one commit, settles it, then finalizes its retention and settles
/// the acknowledgement. Returns the commit's ingress receipt.
pub fn archive(
    canwu: &mut Canwu,
    store: &MemoryArchiveStore,
    books: usize,
) -> FiscalArchiveIngressReceiptV1 {
    let receipt = enqueue_archive(canwu, store, books);
    settle(canwu);
    finalize_fiscal_archive_retention(canwu, store, &receipt).expect("retention finalized");
    settle(canwu);
    receipt
}

/// An evidence archive in memory, for sealing runs whose fiscal state
/// declares payload-required evidence: such a run seals with prepare, store,
/// and commit.
#[derive(Default)]
pub struct MemoryEvidenceArchive {
    segments: RwLock<BTreeMap<String, EvidenceJournalSegment>>,
}

impl ArchiveProvider for MemoryEvidenceArchive {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        Ok(self.segments.read().unwrap().get(segment_id).cloned())
    }
}

impl ArchiveStore for MemoryEvidenceArchive {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError> {
        let id = segment
            .archive
            .as_ref()
            .expect("an archive index")
            .header
            .segment_id
            .clone();
        self.segments.write().unwrap().insert(id, segment.clone());
        Ok(ArchiveStoreOutcome::Stored)
    }
}

/// Seals the compact run's retained evidence through `archive` and returns
/// the sealed segment.
pub fn seal(
    compact: &mut CompactedCanwu,
    archive: &MemoryEvidenceArchive,
) -> EvidenceJournalSegment {
    let prepared = compact
        .prepare_evidence_seal()
        .expect("a seal is prepared")
        .expect("retained evidence exists");
    archive
        .store_evidence_segment(&prepared.segment)
        .expect("the segment is stored");
    compact
        .commit_evidence_seal(&prepared.token, archive)
        .expect("the seal commits");
    prepared.segment
}
