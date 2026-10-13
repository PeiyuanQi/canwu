//! Shared world, fiscal content, and application records for the gap fixtures.
//!
//! Two institutions own fiscal scopes: a treasury with several land scopes and
//! a granary with one. An application plugin owns commission records, the
//! authority bases that fiscal bindings cite; renewing a commission advances
//! its version, and a tick makes a boundary event-bearing without touching
//! fiscal state. The same plugin owns one warrant, a record of a kind the
//! fiscal plugin was not configured to read, and can change whether a person
//! is alive.

// Each integration target intentionally uses a different subset of these helpers.
#![allow(dead_code)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryReceipt,
    BoundarySystemContract, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope,
    CommandRequest, CommandRequestId, CompactedCanwu, CustodyState, DomainRecord,
    DomainRecordClass, DomainRecordDraft, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, Government, GovernmentId,
    IngressClass, IngressPayload, Issuer, KnowledgeHolderRef, KnowledgeRecord, KnowledgeSnapshot,
    LifeState, MapPoint, OrganizationId, PayloadSchema, Person, PersonAvailability, PersonId,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationGranularity, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_fiscal::{
    CompiledFiscalCatalog, FiscalAction, FiscalActionOutcome, FiscalActionRequest,
    FiscalAdoptionStage, FiscalAdoptionState, FiscalAssessmentBasis, FiscalAuthorityBinding,
    FiscalCommutationPolicy, FiscalContentPack, FiscalContentSelection, FiscalCoverageDeclaration,
    FiscalCoverageSelector, FiscalCoverageStatus, FiscalHistoricalMode, FiscalMechanism,
    FiscalObserverBinding, FiscalPackManifest, FiscalPaymentForm, FiscalPeriodDefinition,
    FiscalPlugin, FiscalProvenance, FiscalRegionDefinition, FiscalReport, FiscalReportRecord,
    FiscalRuleDefinition, FiscalScopeBinding, FiscalState, FiscalStateRecord, HistoricalConfidence,
    HistoricalYearWindow, compile_fiscal_content, fiscal_action_command, fiscal_reports,
    fiscal_state_reference, recompute_derived,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
pub const OBSERVER: &str = "observer.principal";

pub const DEPUTY_COMMISSION: &str = "commission.deputy";
pub const SUCCESSOR_COMMISSION: &str = "commission.successor";
pub const AUDITOR_COMMISSION: &str = "commission.auditor";

const COMMISSION_PLUGIN: &str = "fixture-commissions";
const RENEW_COMMISSION_INGRESS: &str = "renew_commission_v1";
const TICK_INGRESS: &str = "tick_v1";
const LIFE_INGRESS: &str = "set_life_v1";

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
        renew.reads = vec![StateKey::core_ingress(), commission_state_key()];
        renew.writes = vec![StateKey::core_person_availability(), commission_state_key()];
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
            fiscal: FiscalPlugin::default()
                .with_authority_basis_kinds([DomainRecordKind::for_type::<Commission>()]),
        }
    }

    #[must_use]
    pub fn plugins(&self) -> [&dyn SimulationPlugin; 2] {
        [&self.commissions, &self.fiscal]
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
        rules: vec![FiscalRuleDefinition {
            id: RULE.to_owned(),
            revision: 1,
            label: "Land levy".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            legal_window: window,
            jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
            subject_scope: "registered_land".to_owned(),
            assessment_basis: FiscalAssessmentBasis::RegisteredLand,
            payment_forms: BTreeSet::from([FiscalPaymentForm::Grain]),
            commutation: FiscalCommutationPolicy::Disabled,
            earmark_ids: BTreeSet::new(),
            provenance_ids: provenance.clone(),
            confidence: HistoricalConfidence::Medium,
        }],
        transitions: Vec::new(),
        coverage: vec![FiscalCoverageDeclaration {
            id: "coverage.a".to_owned(),
            priority: 1,
            selector: FiscalCoverageSelector {
                period_ids: BTreeSet::new(),
                region_ids: BTreeSet::new(),
                mechanisms: BTreeSet::new(),
            },
            status: FiscalCoverageStatus::Supported,
            definition_ids: BTreeSet::from([RULE.to_owned()]),
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
        state.authority_bindings.insert(binding.id.clone(), binding);
    }
    let mut operational_scopes: Vec<_> = (0..TREASURY_SCOPES)
        .map(|index| scope(&treasury_scope(index), treasury()))
        .collect();
    operational_scopes.push(scope(GRANARY_SCOPE, granary()));
    for binding in operational_scopes {
        let adoption_id = binding.id.replacen("scope", "adopt", 1);
        state.adoptions.insert(
            adoption_id.clone(),
            FiscalAdoptionState {
                id: adoption_id,
                rule_id: RULE.to_owned(),
                scope_binding_id: binding.id.clone(),
                stage: FiscalAdoptionStage::Implemented,
                generation: 1,
                changed_at: SimTime::EPOCH,
                source_action_id: None,
            },
        );
        state.scope_bindings.insert(binding.id.clone(), binding);
    }
    state
        .scope_bindings
        .insert(PENDING_SCOPE.to_owned(), scope(PENDING_SCOPE, treasury()));
    state.observer_bindings.insert(
        OBSERVER.to_owned(),
        FiscalObserverBinding {
            id: OBSERVER.to_owned(),
            actor: PRINCIPAL,
            knowledge_holder: KnowledgeHolderRef::Person(PRINCIPAL),
            visible_institutions: BTreeSet::from([treasury()]),
            confidence_per_mille: 1_000,
        },
    );
    state
}

/// The world, the commission records, and the given fiscal state exactly as
/// authored.
#[must_use]
pub fn scenario_from_state(state: FiscalState) -> Scenario {
    let catalog = catalog();
    let state = state.into_record(&catalog).expect("fiscal state record");
    scenario_from_records(catalog.into_record().expect("catalog record"), state)
}

/// The world and the commission records around already encoded fiscal records.
#[must_use]
pub fn scenario_from_records(catalog: DomainRecord, state: DomainRecord) -> Scenario {
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
    domain_records.extend([catalog, state]);
    Scenario {
        start_time: SimTime::EPOCH,
        entities,
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records,
    }
}

/// A scenario whose fiscal state is the base state after `configure`, with
/// its derived fields computed.
#[must_use]
pub fn scenario(configure: impl FnOnce(&mut FiscalState)) -> Scenario {
    let mut state = base_state();
    configure(&mut state);
    recompute_derived(&mut state, &catalog(), SimTime::EPOCH).expect("derived fiscal fields");
    scenario_from_state(state)
}

#[must_use]
pub fn fiscal_state(canwu: &Canwu) -> FiscalState {
    canwu
        .typed_domain_record(&fiscal_state_reference())
        .expect("fiscal state record")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
}

#[must_use]
pub fn fiscal_record_version(canwu: &Canwu) -> u64 {
    canwu
        .typed_domain_record(&fiscal_state_reference())
        .expect("fiscal state record")
        .version
}

#[must_use]
pub fn procedure_revision(canwu: &Canwu) -> u64 {
    fiscal_state(canwu).procedure_revision
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
    canwu
        .boundaries()
        .iter()
        .flat_map(|boundary| &boundary.knowledge_changes)
        .filter(|change| change.holder == KnowledgeHolderRef::Person(actor))
        .flat_map(|change| &change.records)
        .map(|record| {
            let payload = serde_json::from_value(record.payload.clone())
                .expect("a fiscal report record payload");
            (record.clone(), payload)
        })
        .collect()
}

/// One fiscal action that has not been stamped with a procedure revision yet.
#[derive(Clone)]
pub struct Draft {
    action_id: String,
    binding: String,
    action: FiscalAction,
}

impl Draft {
    /// The request an issuer who decided against `revision` sends.
    #[must_use]
    pub fn against(self, revision: u64) -> FiscalActionRequest {
        FiscalActionRequest {
            action_id: self.action_id,
            authority_binding_id: self.binding,
            expected_procedure_revision: revision,
            action: self.action,
        }
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

/// Enqueues the requests as tracked commands due now, in order, so that one
/// boundary admits all of them and the next settles their actions.
pub fn enqueue_requests(canwu: &mut Canwu, commands: &[(PersonId, FiscalActionRequest)]) {
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

/// Enqueues the drafts, every one against the procedure revision the run has
/// now, as issuers who all read the same state would send them.
pub fn enqueue(canwu: &mut Canwu, commands: Vec<(PersonId, Draft)>) {
    let revision = procedure_revision(canwu);
    let commands: Vec<_> = commands
        .into_iter()
        .map(|(issuer, draft)| (issuer, draft.against(revision)))
        .collect();
    enqueue_requests(canwu, &commands);
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
    sealed
        .typed_domain_record(&fiscal_state_reference())
        .expect("fiscal state record")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
}

/// Admits and settles the drafts on a compacted run, as `submit` does on a
/// full one.
pub fn submit_sealed(sealed: &mut CompactedCanwu, commands: Vec<(PersonId, Draft)>) {
    let procedure_revision = sealed_fiscal_state(sealed).procedure_revision;
    let revision = sealed.revision();
    for (offset, (issuer, draft)) in (0_u64..).zip(commands) {
        let command =
            fiscal_action_command(&draft.against(procedure_revision)).expect("fiscal command");
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

#[must_use]
pub fn outcome(canwu: &Canwu, action_id: &str) -> Option<FiscalActionOutcome> {
    fiscal_state(canwu).action_outcomes.get(action_id).cloned()
}

/// The error of the command attempt that carried `action_id`, when admission
/// rejected it.
#[must_use]
pub fn admission_rejection(canwu: &Canwu, action_id: &str) -> Option<CanwuError> {
    canwu.command_attempts().iter().rev().find_map(|attempt| {
        let canwu_api::Command::Plugin { payload, .. } = &attempt.envelope.command else {
            return None;
        };
        if payload["action_id"] != action_id {
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

/// Queues, for the next boundary, a change of whether `person` is alive.
pub fn enqueue_life(canwu: &mut Canwu, person: PersonId, alive: bool) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            COMMISSION_PLUGIN,
            LIFE_INGRESS,
            canwu.time(),
            serde_json::json!({ "person": person.get(), "alive": alive }),
        ))
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
