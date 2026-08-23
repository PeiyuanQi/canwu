use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, Command, CommandEnvelope, CommandRequest, CommandRequestId,
    CompactedCanwu, DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordMutation,
    DomainRecordRef, DomainReference, DomainReferenceTarget, EntityRef, ErrorCode, Government,
    GovernmentId, Issuer, KnowledgeHolderRef, KnowledgeQuery, KnowledgeSnapshot, MapPoint,
    PayloadSchema, Person, PersonId, PluginActionDescriptor, PluginRegistrar, Scenario,
    SimulationPlugin, SimulationView, SystemDirective, Territory, TerritoryId, WorldSnapshot,
};
use canwu_information::{
    AUTHORITY_COMMAND_PRODUCER, AUTHORITY_COMMAND_TYPE, DelegationClaimV1, InformationLifecycle,
    InformationLimitsV1, InformationMutationPlan, InformationOperation,
    InformationOperationEnvelope, InformationOperationId, InformationOperationRecord,
    InformationOperationStatus, InformationOutputKind, InformationOutputSlot, InformationPlugin,
    InformationRecordSet, LifecycleRequest, PLUGIN_NAME, derive_operation_record_ref,
    derive_output_record_ref,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetachedCaseLedger {
    records: BTreeMap<DomainRecordRef, DomainRecord>,
    requests: Vec<LifecycleRequest>,
}

#[derive(Deserialize, Serialize)]
struct DetachedCaseLedgerWire {
    records: Vec<DomainRecord>,
    requests: Vec<LifecycleRequest>,
}

impl DetachedCaseLedger {
    pub fn plan_and_apply(
        &mut self,
        request: &LifecycleRequest,
    ) -> Result<InformationMutationPlan, String> {
        let plan = InformationLifecycle::plan(
            &self.record_set()?,
            request,
            InformationLimitsV1::canonical(),
        )?;
        self.apply(&plan)?;
        self.requests.push(request.clone());
        Ok(plan)
    }

    pub fn plan(&self, request: &LifecycleRequest) -> Result<InformationMutationPlan, String> {
        InformationLifecycle::plan(
            &self.record_set()?,
            request,
            InformationLimitsV1::canonical(),
        )
    }

    pub fn record_set(&self) -> Result<InformationRecordSet, String> {
        InformationRecordSet::from_records(self.records.values().cloned())
    }

    pub fn record(&self, reference: &DomainRecordRef) -> &DomainRecord {
        self.records
            .get(reference)
            .expect("case record should exist")
    }

    pub fn snapshot_json(&self) -> Result<String, String> {
        serde_json::to_string(&DetachedCaseLedgerWire {
            records: self.records.values().cloned().collect(),
            requests: self.requests.clone(),
        })
        .map_err(|error| error.to_string())
    }

    pub fn from_snapshot_json(json: &str) -> Result<Self, String> {
        let wire: DetachedCaseLedgerWire =
            serde_json::from_str(json).map_err(|error| error.to_string())?;
        let mut records = BTreeMap::new();
        for record in wire.records {
            if records.insert(record.reference.clone(), record).is_some() {
                return Err("detached case snapshot contains a duplicate record".to_owned());
            }
        }
        Ok(Self {
            records,
            requests: wire.requests,
        })
    }

    pub fn replay(&self) -> Result<Self, String> {
        let mut replayed = Self::default();
        for request in &self.requests {
            replayed.plan_and_apply(request)?;
        }
        Ok(replayed)
    }

    pub fn domain_records(&self) -> impl Iterator<Item = &DomainRecord> {
        self.records.values()
    }

    fn apply(&mut self, plan: &InformationMutationPlan) -> Result<(), String> {
        for mutation in &plan.mutations {
            match mutation {
                DomainRecordMutation::Create { record } => {
                    if self.records.contains_key(&record.reference) {
                        return Err(format!("duplicate case record {}", record.reference));
                    }
                    self.records.insert(
                        record.reference.clone(),
                        DomainRecord {
                            reference: record.reference.clone(),
                            owner: PLUGIN_NAME.to_owned(),
                            class: DomainRecordClass::Record,
                            version: 1,
                            lifecycle: DomainRecordLifecycle::Active,
                            payload: record.payload.clone(),
                            references: record.references.clone(),
                        },
                    );
                }
                DomainRecordMutation::Update {
                    record,
                    expected_version,
                } => {
                    let current = self
                        .records
                        .get_mut(&record.reference)
                        .ok_or_else(|| format!("missing case record {}", record.reference))?;
                    if current.version != *expected_version {
                        return Err(format!(
                            "case record {} expected version {}, found {}",
                            record.reference, expected_version, current.version
                        ));
                    }
                    current.version += 1;
                    current.payload.clone_from(&record.payload);
                    current.references.clone_from(&record.references);
                }
                DomainRecordMutation::Retire { .. } | DomainRecordMutation::Delete { .. } => {
                    return Err(
                        "the detached case ledger does not apply retire/delete plans".into(),
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct CaseAuthorityPlugin;

#[derive(Clone, Copy)]
struct PersistedResultReplayPlugin;

fn authority_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: AUTHORITY_COMMAND_TYPE.to_owned(),
        description: "Persist one neutral interpretation authority claim".to_owned(),
        payload_schema: PayloadSchema::Any,
        reads: Vec::new(),
        writes: Vec::new(),
    }
}

impl SimulationPlugin for CaseAuthorityPlugin {
    fn name(&self) -> &'static str {
        AUTHORITY_COMMAND_PRODUCER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000c01"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(authority_command_descriptor(), retain_authority_command)
    }
}

impl SimulationPlugin for PersistedResultReplayPlugin {
    fn name(&self) -> &'static str {
        AUTHORITY_COMMAND_PRODUCER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000c01"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            authority_command_descriptor(),
            consume_persisted_interpretation_result,
        )
    }
}

#[allow(clippy::unnecessary_wraps)]
fn retain_authority_command(
    _view: &SimulationView<'_>,
    _context: &canwu_api::CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(Vec::new())
}

fn consume_persisted_interpretation_result(
    _view: &SimulationView<'_>,
    _context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if payload.pointer("/result/format").and_then(Value::as_str)
        == Some("neutral_interpretation_result_v1")
    {
        Ok(Vec::new())
    } else {
        Err(CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay requires the persisted interpretation result",
        ))
    }
}

pub fn verify_authoritative_operation_roundtrip<F>(
    namespace: &str,
    seed: &DetachedCaseLedger,
    request: &LifecycleRequest,
    expected: &DetachedCaseLedger,
    authority_claim: Option<DelegationClaimV1>,
    verify_knowledge: F,
) -> Result<(), String>
where
    F: FnOnce(&Canwu) -> Result<(), String>,
{
    let (operation_id, envelope) = authoritative_envelope(namespace, request)?;
    let has_external_authority = authority_claim.is_some();
    let scenario = Scenario {
        start_time: canwu_api::SimTime::from_minutes(1_000),
        world: case_world(),
        knowledge: KnowledgeSnapshot::default(),
        domain_records: seed.domain_records().cloned().collect(),
    };
    let information = InformationPlugin;
    let authority = CaseAuthorityPlugin;
    let plugins: Vec<&dyn SimulationPlugin> = if authority_claim.is_some() {
        vec![&information, &authority]
    } else {
        vec![&information]
    };
    let mut canwu =
        Canwu::new_with_plugins(0xCA5E_2026, scenario.clone(), &plugins).map_err(stringify)?;
    enqueue_authority_and_operation(&mut canwu, authority_claim, &envelope)?;
    settle_until_terminal(&mut canwu, &operation_id)?;
    verify_completed_records(&canwu, &operation_id, expected)?;
    verify_knowledge(&canwu)?;

    // Close one later boundary so every retained event belongs to a completed
    // evidence cut before compact sealing.
    settle(&mut canwu)?;
    let snapshot = canwu.snapshot();
    let replay_consumer = PersistedResultReplayPlugin;
    let replay_plugins: Vec<&dyn SimulationPlugin> = if has_external_authority {
        vec![&information, &replay_consumer]
    } else {
        vec![&information]
    };
    verify_snapshot_and_replay(&canwu, scenario, &plugins, &replay_plugins, &snapshot)?;
    verify_compact_reconstruction(canwu, &plugins, &snapshot)
}

pub fn assert_authoritative_knowledge(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
    expected_schemas: &[&str],
) -> Result<(), String> {
    let result = canwu
        .admin_query_knowledge(holder.clone(), &KnowledgeQuery::default())
        .map_err(stringify)?;
    let mut actual = result
        .records
        .iter()
        .map(|record| record.schema.kind.name.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    let mut expected = expected_schemas.to_vec();
    expected.sort_unstable();
    if actual != expected {
        return Err(format!(
            "authoritative knowledge mismatch for {holder:?}: expected {expected:?}, found {actual:?}"
        ));
    }
    Ok(())
}

fn authoritative_envelope(
    namespace: &str,
    request: &LifecycleRequest,
) -> Result<(InformationOperationId, InformationOperationEnvelope), String> {
    let operation_id = InformationOperationId::new(namespace, "authoritative-operation");
    let (operation_kind, output_slots) = match request {
        LifecycleRequest::TransitionRelease { .. } => ("transition_release", Vec::new()),
        LifecycleRequest::RecordInterpretation { binding, .. } => {
            let output = InformationOutputSlot {
                index: 0,
                name: "result".to_owned(),
                kind: InformationOutputKind::Interpretation,
            };
            if derive_output_record_ref(&operation_id, &output) != *binding.reference.as_untyped() {
                return Err(
                    "authoritative interpretation binding is not derived from its operation"
                        .to_owned(),
                );
            }
            ("record_interpretation", vec![output])
        }
        _ => {
            return Err(
                "the public case authoritative proof does not support this operation".to_owned(),
            );
        }
    };
    Ok((
        operation_id.clone(),
        InformationOperationEnvelope {
            id: operation_id.clone(),
            operation_version: 1,
            operation_kind: operation_kind.to_owned(),
            output_slots,
            lineage: Vec::new(),
            operation: InformationOperation {
                request: request.clone(),
            },
        },
    ))
}

fn enqueue_authority_and_operation(
    canwu: &mut Canwu,
    authority_claim: Option<DelegationClaimV1>,
    envelope: &InformationOperationEnvelope,
) -> Result<(), String> {
    let mut request_id = 1_u64;
    if let Some(claim) = authority_claim {
        enqueue_plugin_command(
            canwu,
            request_id,
            AUTHORITY_COMMAND_PRODUCER,
            AUTHORITY_COMMAND_TYPE,
            json!({
                "claim": claim,
                "result": {"format": "neutral_interpretation_result_v1"}
            }),
        )?;
        settle(canwu)?;
        request_id += 1;
    }
    enqueue_plugin_command(
        canwu,
        request_id,
        PLUGIN_NAME,
        canwu_information::INFORMATION_COMMAND,
        serde_json::to_value(envelope).map_err(stringify)?,
    )
}

fn settle_until_terminal(
    canwu: &mut Canwu,
    operation_id: &InformationOperationId,
) -> Result<(), String> {
    for _ in 0..8 {
        settle(canwu)?;
        if canwu
            .typed_domain_record(&derive_operation_record_ref(operation_id))
            .and_then(|record| record.decode_payload::<InformationOperationRecord>().ok())
            .is_some_and(|operation| operation.status.is_terminal())
        {
            return Ok(());
        }
    }
    Err("authoritative case operation did not reach a terminal state".to_owned())
}

fn verify_completed_records(
    canwu: &Canwu,
    operation_id: &InformationOperationId,
    expected: &DetachedCaseLedger,
) -> Result<(), String> {
    let operation = canwu
        .typed_domain_record(&derive_operation_record_ref(operation_id))
        .ok_or_else(|| "authoritative case operation record is missing".to_owned())?
        .decode_payload::<InformationOperationRecord>()
        .map_err(stringify)?;
    if operation.status != InformationOperationStatus::Completed {
        return Err(format!(
            "authoritative case operation did not complete: {:?}",
            operation.status
        ));
    }
    for record in expected.domain_records() {
        if canwu.domain_record(&record.reference) != Some(record) {
            return Err(format!(
                "authoritative case result diverged for {}",
                record.reference
            ));
        }
    }
    Ok(())
}

fn verify_snapshot_and_replay(
    canwu: &Canwu,
    scenario: Scenario,
    restore_plugins: &[&dyn SimulationPlugin],
    replay_plugins: &[&dyn SimulationPlugin],
    snapshot: &canwu_api::SimulationSnapshot,
) -> Result<(), String> {
    let snapshot_json = canwu.snapshot_json().map_err(stringify)?;
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot_json, restore_plugins)
        .map_err(stringify)?;
    if restored.snapshot() != *snapshot {
        return Err("authoritative case snapshot restore diverged".to_owned());
    }
    let replayed = Canwu::replay_from_journal(scenario, replay_plugins, &canwu.replay_journal())
        .map_err(stringify)?;
    if replayed.snapshot() != *snapshot {
        return Err("authoritative case exact replay diverged".to_owned());
    }
    Ok(())
}

fn verify_compact_reconstruction(
    canwu: Canwu,
    plugins: &[&dyn SimulationPlugin],
    snapshot: &canwu_api::SimulationSnapshot,
) -> Result<(), String> {
    let mut compact = canwu.into_compacted().map_err(stringify)?;
    let segment = compact
        .seal_evidence()
        .map_err(stringify)?
        .ok_or_else(|| "authoritative case produced no compact evidence segment".to_owned())?;
    let checkpoint = compact.checkpoint().map_err(stringify)?;
    if compact
        .snapshot_with_segments(vec![segment.clone()])
        .map_err(stringify)?
        != *snapshot
    {
        return Err("authoritative case compact reconstruction diverged".to_owned());
    }
    let restored_compact = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        checkpoint,
        vec![segment],
        plugins,
    )
    .map_err(stringify)?;
    if restored_compact
        .snapshot_with_segments(Vec::new())
        .map_err(stringify)?
        != *snapshot
    {
        return Err("authoritative compact restore diverged".to_owned());
    }
    Ok(())
}

fn enqueue_plugin_command(
    canwu: &mut Canwu,
    request_id: u64,
    plugin: &str,
    command: &str,
    payload: Value,
) -> Result<(), String> {
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("public-case-runtime-proof".to_owned()),
                    Command::Plugin {
                        plugin: plugin.to_owned(),
                        command: command.to_owned(),
                        payload,
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .map(|_| ())
        .map_err(stringify)
}

fn settle(canwu: &mut Canwu) -> Result<(), String> {
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .map(|_| ())
        .map_err(stringify)
}

fn case_world() -> WorldSnapshot {
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    WorldSnapshot {
        people: (1..=1_000)
            .map(|id| Person {
                id: PersonId::new(id),
                name: format!("Case Holder {id:04}"),
                government,
                current_location: territory,
                roles: vec!["case_holder".to_owned()],
                transit: None,
            })
            .collect(),
        governments: vec![Government {
            id: government,
            name: "Case Institution".to_owned(),
            capital: territory,
        }],
        territories: vec![Territory {
            id: territory,
            name: "Case Node".to_owned(),
            controller: government,
            position: MapPoint { x: 0.0, y: 0.0 },
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    }
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn holder_reference(role: &str, holder: &KnowledgeHolderRef) -> DomainReference {
    entity_reference(role, holder_entity(holder))
}

pub fn entity_reference(role: &str, entity: EntityRef) -> DomainReference {
    DomainReference {
        role: role.to_owned(),
        target: match entity {
            EntityRef::Domain(reference) => DomainReferenceTarget::Domain(reference),
            core => DomainReferenceTarget::Core(core),
        },
    }
}

fn holder_entity(holder: &KnowledgeHolderRef) -> EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}
