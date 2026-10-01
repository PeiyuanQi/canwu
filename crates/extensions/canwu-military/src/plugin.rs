use crate::model::*;
use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, Command, CommandContext, CommandIngress, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, EntityRef, ErrorCode, EvidenceRef,
    IngressClass, IngressPayload, Issuer, KnowledgeHolderRef, KnowledgeLimitsV1, KnowledgeOrigin,
    KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, LifeState, PayloadSchema, PersonId, PluginActionDescriptor,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, RandomOperationTarget,
    RandomStreamKey, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MILITARY_COMMAND: &str = "military_command_v1";
pub const MILITARY_COMMAND_INGRESS: &str = "military_command_v1";
pub const MILITARY_PROVIDER_ACK_INGRESS: &str = "military_provider_ack_v1";
pub const MILITARY_REPORT_KNOWLEDGE: &str = "military_report";
/// Event recorded when phase 7 rejects a military command or provider
/// acknowledgement instead of failing the boundary.
pub const MILITARY_REJECTION_EVENT: &str = "canwu.military.ingress_rejected.v1";
const VERSION: &str = "0.1.0";
const SEMANTIC_HASH: &str = "b8c3f8a2a95ff7becd1d61190fdb0e168700120c6de911a325c28084265cc765";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AdmittedCommand {
    envelope: MilitaryCommandEnvelope,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ProviderAck {
    outcome: ProviderOutcome,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MilitaryPlugin;

impl SimulationPlugin for MilitaryPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }
    fn version(&self) -> &'static str {
        VERSION
    }
    fn semantic_hash(&self) -> &'static str {
        SEMANTIC_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for schema in [
            DomainRecordSchema::for_record::<MilitaryCatalogRecord>(),
            DomainRecordSchema::for_record::<ForceStateRecord>(),
            DomainRecordSchema::for_record::<OperationStateRecord>(),
            DomainRecordSchema::for_record::<CombatStateRecord>(),
            DomainRecordSchema::for_record::<OccupationStateRecord>(),
            DomainRecordSchema::for_record::<MilitaryKnowledgeRecord>(),
            DomainRecordSchema::for_record::<ProviderOutcomeRecord>(),
            DomainRecordSchema::for_record::<MilitaryLedgerRecord>(),
        ] {
            registrar.register_record_schema(schema)?;
        }

        registrar.register_knowledge_schema(report_schema())?;
        let mut command_reads = military_state_keys();
        command_reads.push(StateKey::core_person_availability());
        registrar.register_command(
            PluginActionDescriptor {
                name: MILITARY_COMMAND.to_owned(),
                description: "Admit one military domain command".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: command_reads,
                writes: Vec::new(),
            },
            admit_command,
        )?;
        // Only the command handler and the plugin's own ticks queue this
        // packet; hosts cannot author it.
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: MILITARY_COMMAND_INGRESS.to_owned(),
            description: "Apply one admitted military command".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: MILITARY_PROVIDER_ACK_INGRESS.to_owned(),
            description: "Acknowledge one exact pending military provider effect".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;

        let mut apply = BoundarySystemContract::new(
            "apply-military-ingress-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = military_state_keys();
        apply.reads.push(StateKey::core_ingress());
        apply.writes = military_state_keys();
        apply.visibility = StateVisibility::SameBoundary;
        apply.random_streams = vec![military_random_stream()];
        apply.emits = vec![
            "canwu.military.transition_applied.v1".to_owned(),
            MILITARY_REJECTION_EVENT.to_owned(),
        ];
        apply.plugin_ingress_targets = vec![canwu_api::PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: MILITARY_COMMAND_INGRESS.to_owned(),
        }];
        registrar.register_boundary_system(apply, apply_ingress)?;
        let mut report = BoundarySystemContract::new(
            "materialize-military-reports-v1",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        report.reads = military_state_keys();
        report.reads.push(StateKey::core_person_availability());
        report.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: report_schema_id(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        report.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(report, materialize_reports)
    }
}

pub fn military_plugin() -> MilitaryPlugin {
    MilitaryPlugin
}

pub fn military_command(command: MilitaryCommand) -> Result<Command, CanwuError> {
    let envelope = MilitaryCommandEnvelope {
        input_digest: input_digest(&command)?,
        command,
    };
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: MILITARY_COMMAND.to_owned(),
        payload: serde_json::to_value(envelope).map_err(encode)?,
    })
}

pub fn enqueue_provider_outcome(
    canwu: &mut Canwu,
    due_at: SimTime,
    outcome: ProviderOutcome,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    Ok(canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        PLUGIN_NAME,
        MILITARY_PROVIDER_ACK_INGRESS,
        due_at,
        serde_json::to_value(ProviderAck { outcome }).map_err(encode)?,
    ))?)
}

pub fn military_random_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN_NAME, "military-operation", 1)
}
pub fn report_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAMESPACE, MILITARY_REPORT_KNOWLEDGE),
        1,
    )
}

fn report_schema() -> canwu_api::PluginKnowledgeSchema {
    canwu_api::PluginKnowledgeSchema {
        id: report_schema_id(),
        schema_hash: "6e4bf5e8cf2dff7fddc8c2e75d0d5f0bbdf43ce43d6e19e2a6c3ef0a6d8f4c1b".to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: vec![KnowledgeSubjectSchema {
            role: "force".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Domain(
                DomainRecordKind::for_type::<ForceStateRecord>(),
            )],
            required: true,
            multiple: false,
        }],
    }
}

fn military_state_keys() -> Vec<StateKey> {
    [
        DomainRecordSchema::for_record::<MilitaryCatalogRecord>(),
        DomainRecordSchema::for_record::<ForceStateRecord>(),
        DomainRecordSchema::for_record::<OperationStateRecord>(),
        DomainRecordSchema::for_record::<CombatStateRecord>(),
        DomainRecordSchema::for_record::<OccupationStateRecord>(),
        DomainRecordSchema::for_record::<ProviderOutcomeRecord>(),
        DomainRecordSchema::for_record::<MilitaryLedgerRecord>(),
    ]
    .into_iter()
    .map(|s| s.state_key())
    .collect()
}

fn admit_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(err(
            ErrorCode::MixedCommandIngress,
            "military commands require canonical command ingress",
        ));
    }
    let envelope: MilitaryCommandEnvelope = decode(payload, "military command")?;
    let command_digest = input_digest(&envelope.command)?;
    if envelope.input_digest != command_digest {
        return Err(err(
            ErrorCode::InvalidPayload,
            "military command semantic digest mismatch",
        ));
    }
    let affected = validate_command(view, context, &envelope.command)?;
    let ledger = view
        .typed_domain_record(&ledger_reference())?
        .map(DomainRecord::decode_payload::<MilitaryLedgerRecord>)
        .transpose()?;
    if already_settled(ledger.as_ref(), &envelope.command, &command_digest)? {
        return Ok(Vec::new());
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: MILITARY_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedCommand { envelope }).map_err(encode)?,
        affected,
    }])
}

/// Checks authority and static validity. Every error uses a code the engine
/// records as a rejected command attempt, so a bad command never blocks the
/// queue. Checks against changing force state run again in phase 7.
fn validate_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    command: &MilitaryCommand,
) -> Result<Vec<EntityRef>, CanwuError> {
    let operation = command_operation(command);
    if operation.as_str().is_empty() {
        return Err(err(
            ErrorCode::InvalidPayload,
            "military operation key is empty",
        ));
    }
    let Issuer::Actor(actor) = context.issuer else {
        return Err(err(
            ErrorCode::InvalidAuthority,
            "military commands require an actor issuer",
        ));
    };
    let mut affected = Vec::new();
    match command {
        MilitaryCommand::AdvanceTick { .. } => {
            return Err(err(
                ErrorCode::InvalidAuthority,
                "military ticks are scheduled by the plugin and cannot be sent as commands",
            ));
        }
        MilitaryCommand::CreateForce { commander, .. } => {
            if *commander != Some(actor) {
                return Err(err(
                    ErrorCode::InvalidAuthority,
                    "a new military force must be commanded by the actor who creates it",
                ));
            }
        }
        MilitaryCommand::AssignCommander { commander, .. } => {
            if view
                .person_availability(*commander)?
                .is_some_and(|availability| availability.life == LifeState::Dead)
            {
                return Err(err(
                    ErrorCode::InvalidPayload,
                    "a dead person cannot command a military force",
                ));
            }
            // The kernel rejects the command if this person does not exist.
            affected.push(EntityRef::Person(*commander));
        }
        MilitaryCommand::SetOccupationPolicy {
            occupation,
            security_per_mille,
            collaboration_per_mille,
            extraction_burden_per_mille,
            ..
        } => {
            for (value, label) in [
                (security_per_mille, "security"),
                (collaboration_per_mille, "collaboration"),
                (extraction_burden_per_mille, "extraction burden"),
            ] {
                validate_per_mille(*value, label)
                    .map_err(|error| err(ErrorCode::ValueOutOfRange, error.message))?;
            }
            require_occupation_commander(view, occupation, actor)?;
        }
        MilitaryCommand::MilitaryAdministrationAction { occupation, .. } => {
            require_occupation_commander(view, occupation, actor)?;
        }
        _ => {}
    }
    if let Some(force) = command_force(command) {
        if !matches!(command, MilitaryCommand::CreateForce { .. }) {
            require_force_commander(view, force, actor)?;
        }
    }
    if let Some(catalog_record) = view.typed_domain_record(&catalog_reference())? {
        let catalog = catalog_record.decode_payload::<MilitaryCatalogRecord>()?;
        catalog.ruleset.validate().map_err(|error| {
            err(
                ErrorCode::InvalidPayload,
                format!("installed military ruleset is invalid: {}", error.message),
            )
        })?;
        let branch = match command {
            MilitaryCommand::CreateForce { branch, .. }
            | MilitaryCommand::Recruit { branch, .. } => Some(branch),
            _ => None,
        };
        if let Some(branch) = branch {
            if !catalog.ruleset.branch_profiles.contains_key(branch) {
                return Err(err(
                    ErrorCode::InvalidPayload,
                    "military branch is not present in the active ruleset",
                ));
            }
        }
        let tactic = match command {
            MilitaryCommand::OrderMarch { tactic, .. }
            | MilitaryCommand::PlanOperation { tactic, .. }
            | MilitaryCommand::PrepareAmbush { tactic, .. } => Some(tactic),
            _ => None,
        };
        if let Some(tactic) = tactic {
            if !catalog.ruleset.tactics.contains_key(tactic) {
                return Err(err(
                    ErrorCode::InvalidPayload,
                    "military tactic is not present in the active ruleset",
                ));
            }
        }
    }
    Ok(affected)
}

fn require_force_commander(
    view: &SimulationView<'_>,
    force: &ForceId,
    actor: PersonId,
) -> Result<(), CanwuError> {
    let state = view
        .typed_domain_record(&force_reference(force))?
        .ok_or_else(|| err(ErrorCode::EntityNotFound, "military force does not exist"))?
        .decode_payload::<ForceStateRecord>()?;
    if state.commander != Some(actor) {
        return Err(err(
            ErrorCode::InvalidAuthority,
            "actor does not command this force",
        ));
    }
    Ok(())
}

fn require_occupation_commander(
    view: &SimulationView<'_>,
    occupation: &OccupationId,
    actor: PersonId,
) -> Result<(), CanwuError> {
    let state = view
        .typed_domain_record(&occupation_reference(occupation))?
        .ok_or_else(|| {
            err(
                ErrorCode::EntityNotFound,
                "military occupation does not exist",
            )
        })?
        .decode_payload::<OccupationStateRecord>()?;
    require_force_commander(view, &state.occupying_force, actor)
}

/// Returns true when this exact command already has an outcome under its
/// key. Any other use of a settled or pending key is an idempotency conflict.
fn already_settled(
    ledger: Option<&MilitaryLedger>,
    command: &MilitaryCommand,
    command_digest: &str,
) -> Result<bool, CanwuError> {
    let key = command_operation(command);
    let Some(ledger) = ledger else {
        return Ok(false);
    };
    match ledger.outcomes.get(key) {
        Some(existing) if existing.input_digest == command_digest => Ok(true),
        None if !ledger.pending.contains_key(key) => Ok(false),
        _ => Err(err(
            ErrorCode::IdempotencyConflict,
            "military operation key was reused with different input",
        )),
    }
}

/// Military writes staged by one phase-7 pass. Later packets in the pass read
/// the writes of earlier ones, and each record receives one mutation.
struct Staging<'v, 'a> {
    view: &'v SimulationView<'a>,
    directives: Vec<BoundaryDirective>,
    mutations: BTreeMap<DomainRecordRef, usize>,
}

type StagingCheckpoint = (Vec<BoundaryDirective>, BTreeMap<DomainRecordRef, usize>);

impl<'v, 'a> Staging<'v, 'a> {
    fn new(view: &'v SimulationView<'a>) -> Self {
        Self {
            view,
            directives: Vec::new(),
            mutations: BTreeMap::new(),
        }
    }

    /// Reads a record as staged so far. A staged record keeps the version it
    /// had before this pass (0 when created in it), so a second change in the
    /// same pass derives the same next revision.
    fn record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Result<Option<DomainRecord>, CanwuError> {
        let reference = reference.as_untyped();
        let Some(index) = self.mutations.get(reference) else {
            return Ok(self.view.domain_record(reference)?.cloned());
        };
        let (draft, version) = match self.directives.get(*index) {
            Some(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create { record },
                ..
            }) => (record, 0),
            Some(BoundaryDirective::MutateRecord {
                mutation:
                    DomainRecordMutation::Update {
                        record,
                        expected_version,
                    },
                ..
            }) => (record, *expected_version),
            _ => return Err(staging_error()),
        };
        Ok(Some(DomainRecord {
            reference: draft.reference.clone(),
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload.clone(),
            references: draft.references.clone(),
        }))
    }

    fn create<T: DomainRecordType>(
        &mut self,
        reference: TypedDomainRecordRef<T>,
        payload: &T::Payload,
        summary: &str,
    ) -> Result<(), CanwuError>
    where
        T::Payload: Serialize,
    {
        if self.record(&reference)?.is_some() {
            return Err(err(
                ErrorCode::DuplicateDomainRecord,
                "military record already exists",
            ));
        }
        let record = DomainRecordDraft::from_typed(reference, payload)?;
        self.mutations
            .insert(record.reference.clone(), self.directives.len());
        self.directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record },
            summary: summary.to_owned(),
        });
        Ok(())
    }

    fn upsert<T: DomainRecordType>(
        &mut self,
        reference: TypedDomainRecordRef<T>,
        payload: &T::Payload,
        summary: &str,
    ) -> Result<(), CanwuError>
    where
        T::Payload: Serialize,
    {
        let record = DomainRecordDraft::from_typed(reference, payload)?;
        if let Some(index) = self.mutations.get(&record.reference) {
            return match self.directives.get_mut(*index) {
                Some(BoundaryDirective::MutateRecord {
                    mutation:
                        DomainRecordMutation::Create { record: staged }
                        | DomainRecordMutation::Update { record: staged, .. },
                    ..
                }) => {
                    *staged = record;
                    Ok(())
                }
                _ => Err(staging_error()),
            };
        }
        let current = self.view.domain_record(&record.reference)?.ok_or_else(|| {
            err(
                ErrorCode::DomainRecordNotFound,
                "military record is unavailable",
            )
        })?;
        let expected_version = current.version;
        self.mutations
            .insert(record.reference.clone(), self.directives.len());
        self.directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record,
                expected_version,
            },
            summary: summary.to_owned(),
        });
        Ok(())
    }

    fn push(&mut self, directive: BoundaryDirective) {
        self.directives.push(directive);
    }

    fn checkpoint(&self) -> StagingCheckpoint {
        (self.directives.clone(), self.mutations.clone())
    }

    fn restore(&mut self, (directives, mutations): StagingCheckpoint) {
        self.directives = directives;
        self.mutations = mutations;
    }

    fn ledger(&self) -> Result<Option<MilitaryLedger>, CanwuError> {
        self.record(&ledger_reference())?
            .map(|record| record.decode_payload::<MilitaryLedgerRecord>())
            .transpose()
    }

    fn created_count(&self, kind: &DomainRecordKind) -> usize {
        self.mutations
            .iter()
            .filter(|(reference, index)| {
                reference.kind == *kind
                    && matches!(
                        self.directives.get(**index),
                        Some(BoundaryDirective::MutateRecord {
                            mutation: DomainRecordMutation::Create { .. },
                            ..
                        })
                    )
            })
            .count()
    }
}

fn staging_error() -> CanwuError {
    err(
        ErrorCode::InvalidBoundary,
        "staged military mutation is inconsistent",
    )
}

/// Rejects a command that would create one more record of a kind that
/// already holds `MAX_RECORDS` records, counting `reserved` future records.
fn ensure_record_capacity<T: DomainRecordType>(
    staging: &Staging<'_, '_>,
    reserved: usize,
) -> Result<(), CanwuError> {
    let kind = DomainRecordKind::for_type::<T>();
    let stored = staging
        .view
        .domain_records_of_kind(&kind, MAX_RECORDS)?
        .len();
    if stored + staging.created_count(&kind) + reserved >= MAX_RECORDS {
        return Err(err(
            ErrorCode::ValueOutOfRange,
            format!("military {} records are at MAX_RECORDS", kind.name),
        ));
    }
    Ok(())
}

fn apply_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut staging = Staging::new(view);
    for id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != PLUGIN_NAME {
            continue;
        }
        if packet_type == MILITARY_COMMAND_INGRESS {
            let admitted: AdmittedCommand = decode(payload, "admitted military command")?;
            let command = &admitted.envelope.command;
            // A plugin-scheduled tick that fails is a broken invariant, so it
            // still fails the boundary.
            if matches!(command, MilitaryCommand::AdvanceTick { .. }) {
                apply_command(&mut staging, context, command)?;
                continue;
            }
            let checkpoint = staging.checkpoint();
            if let Err(error) = apply_command(&mut staging, context, command) {
                staging.restore(checkpoint);
                reject_command(&mut staging, context, command, &error)?;
            }
        }
        if packet_type == MILITARY_PROVIDER_ACK_INGRESS {
            let checkpoint = staging.checkpoint();
            let result = decode::<ProviderAck>(payload, "military provider acknowledgement")
                .and_then(|ack| apply_ack(&mut staging, context, &ack.outcome));
            if let Err(error) = result {
                staging.restore(checkpoint);
                record_rejection(
                    &mut staging,
                    format!("military provider acknowledgement rejected: {error}"),
                );
            }
        }
    }
    Ok(BoundaryProposal {
        directives: staging.directives,
        ..BoundaryProposal::default()
    })
}

/// Records a rejected command under its key when the key is free, so a
/// resend is a no-op, and emits a rejection event either way.
fn reject_command(
    staging: &mut Staging<'_, '_>,
    context: &BoundaryContext,
    command: &MilitaryCommand,
    error: &CanwuError,
) -> Result<(), CanwuError> {
    let key = command_operation(command);
    let key_in_use = staging.ledger()?.is_some_and(|ledger| {
        ledger.outcomes.contains_key(key) || ledger.pending.contains_key(key)
    });
    if !key_in_use {
        record_command_outcome(
            staging,
            command,
            input_digest(command)?,
            context.at,
            OutcomeDisposition::Rejected,
            &error.to_string(),
        )?;
    }
    record_rejection(staging, format!("military command {key} rejected: {error}"));
    Ok(())
}

fn record_rejection(staging: &mut Staging<'_, '_>, summary: String) {
    staging.push(BoundaryDirective::Emit {
        event_type: MILITARY_REJECTION_EVENT.to_owned(),
        summary,
        affected: Vec::new(),
    });
}

/// Rechecks a command's expected force revision against the force as staged
/// in this pass.
fn check_force_revision(
    staging: &Staging<'_, '_>,
    command: &MilitaryCommand,
) -> Result<(), CanwuError> {
    let (Some(force), Some(expected)) =
        (command_force(command), command_expected_revision(command))
    else {
        return Ok(());
    };
    let state = staging
        .record(&force_reference(force))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "force is unavailable"))?
        .decode_payload::<ForceStateRecord>()?;
    if state.meta.revision != expected {
        return Err(err(
            ErrorCode::DomainRecordVersionConflict,
            "military force revision is stale",
        ));
    }
    Ok(())
}

fn apply_command(
    staging: &mut Staging<'_, '_>,
    context: &BoundaryContext,
    command: &MilitaryCommand,
) -> Result<(), CanwuError> {
    let at = context.at;
    let command_digest = input_digest(command)?;
    let internal_tick = matches!(command, MilitaryCommand::AdvanceTick { .. });
    if !internal_tick {
        if already_settled(staging.ledger()?.as_ref(), command, &command_digest)? {
            return Ok(());
        }
        check_force_revision(staging, command)?;
    }
    match command {
        MilitaryCommand::CreateForce {
            force,
            owner,
            location,
            authorized_strength,
            initial_strength,
            branch,
            commander,
            ..
        } => {
            if staging.record(&force_reference(force))?.is_some() {
                return Err(err(
                    ErrorCode::DuplicateDomainRecord,
                    "force already exists",
                ));
            }
            ensure_record_capacity::<ForceStateRecord>(staging, 0)?;
            let initial_strength = initial_strength.unwrap_or(*authorized_strength);
            if initial_strength > *authorized_strength {
                return Err(err(
                    ErrorCode::InvalidPayload,
                    "initial strength exceeds authorized strength",
                ));
            }
            let unit = SubunitState {
                id: SubunitId::new(format!("{}:initial", force.as_str()))?,
                branch: branch.clone(),
                strength: initial_strength,
                training_per_mille: 0,
                equipment_per_mille: 0,
                fatigue_per_mille: 0,
                status: SubunitStatus::Active,
            };
            let mut state = ForceState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: force.clone(),
                owner: owner.clone(),
                formation_parent: None,
                location: location.clone(),
                commander: *commander,
                subunits: BTreeMap::from([(unit.id.clone(), unit)]),
                authorized_strength: *authorized_strength,
                actual_strength: initial_strength,
                training_per_mille: 0,
                equipment_per_mille: 0,
                fatigue_per_mille: 0,
                supply_per_mille: 1_000,
                morale_per_mille: 500,
                discipline_per_mille: 500,
                cohesion_per_mille: 500,
                loyalty_per_mille: 500,
                casualties: 0,
                missing: 0,
                prisoners: 0,
                deserters: 0,
                replacements_pending: 0,
                transport_capacity: 0,
                active_operation: None,
                active_order: None,
                prepared_ambush: None,
                status: ForceStatus::Forming,
            };
            state.meta = MilitaryRecordMeta::new(1, at, &state)?;
            state.validate()?;
            staging.create(force_reference(force), &state, "Create military force")?;
        }
        MilitaryCommand::AssignCommander {
            force, commander, ..
        } => update_force(staging, force, at, |s| {
            s.commander = Some(*commander);
            Ok(())
        })?,
        MilitaryCommand::Recruit {
            force,
            subunit,
            branch,
            quantity,
            ..
        } => update_force(staging, force, at, |s| {
            if s.subunits.contains_key(subunit) {
                return Err(err(
                    ErrorCode::IdempotencyConflict,
                    "subunit already exists",
                ));
            }
            if s.subunits.len() >= MAX_SUBUNITS {
                return Err(err(
                    ErrorCode::ValueOutOfRange,
                    "force already has MAX_SUBUNITS subunits",
                ));
            }
            let recruitable =
                (*quantity).min(s.authorized_strength.saturating_sub(s.actual_strength));
            if recruitable == 0 {
                return Err(err(
                    ErrorCode::InvalidDecision,
                    "force has no remaining recruitment capacity",
                ));
            }
            s.subunits.insert(
                subunit.clone(),
                SubunitState {
                    id: subunit.clone(),
                    branch: branch.clone(),
                    strength: recruitable,
                    training_per_mille: 0,
                    equipment_per_mille: 0,
                    fatigue_per_mille: 0,
                    status: SubunitStatus::Active,
                },
            );
            s.actual_strength = s.actual_strength.saturating_add(recruitable);
            Ok(())
        })?,
        MilitaryCommand::TrainAndEquip {
            force,
            training_delta,
            equipment_delta,
            ..
        } => update_force(staging, force, at, |s| {
            s.training_per_mille = s
                .training_per_mille
                .saturating_add(*training_delta)
                .min(1_000);
            s.equipment_per_mille = s
                .equipment_per_mille
                .saturating_add(*equipment_delta)
                .min(1_000);
            Ok(())
        })?,
        MilitaryCommand::OrderMarch {
            force,
            operation_id,
            destination,
            objective,
            opposing_force,
            ..
        } => {
            let current = force_state(staging, force)?;
            if let Some(opponent) = opposing_force {
                require_opposing_force(staging, opponent)?;
                ensure_derived_ids(operation_id)?;
            }
            ensure_record_capacity::<OperationStateRecord>(staging, 0)?;
            let operation = OperationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: operation_id.clone(),
                key: command_operation(command).clone(),
                owner: current.owner,
                objective: objective.clone(),
                kind: "march".to_owned(),
                forces: vec![force.clone()],
                opposing_force: opposing_force.clone(),
                phase: OperationPhase::Moving,
                from: current.location,
                destination: destination.clone(),
                route_digest: digest(&(force, destination))?,
                terrain: String::new(),
                weather: String::new(),
                started_at: at,
                due_at: at
                    .checked_add(SimDuration::minutes(1))
                    .ok_or_else(|| err(ErrorCode::InvalidDuration, "military march overflow"))?,
                command_delay_minutes: 0,
                supply_line: None,
                exit_condition: String::new(),
            };
            operation.validate()?;
            update_force(staging, force, at, |s| {
                s.active_operation = Some(operation_id.clone());
                s.status = ForceStatus::Moving;
                Ok(())
            })?;
            staging.create(
                operation_reference(operation_id),
                &operation,
                "Order military march",
            )?;
            schedule_tick(
                staging,
                SimDuration::minutes(1),
                Some(operation_id.clone()),
                None,
                command_operation(command).clone(),
            )?;
        }
        MilitaryCommand::PlanOperation {
            operation_id,
            owner,
            objective,
            force,
            from,
            destination,
            opposing_force,
            ..
        } => {
            if let Some(opponent) = opposing_force {
                require_opposing_force(staging, opponent)?;
            }
            ensure_record_capacity::<OperationStateRecord>(staging, 0)?;
            let op = OperationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: operation_id.clone(),
                key: command_operation(command).clone(),
                owner: owner.clone(),
                objective: objective.clone(),
                kind: "strategic".to_owned(),
                forces: vec![force.clone()],
                opposing_force: opposing_force.clone(),
                phase: OperationPhase::Planned,
                from: from.clone(),
                destination: destination.clone(),
                route_digest: digest(&(from, destination))?,
                terrain: String::new(),
                weather: String::new(),
                started_at: at,
                due_at: at,
                command_delay_minutes: 0,
                supply_line: None,
                exit_condition: String::new(),
            };
            op.validate()?;
            staging.create(
                operation_reference(operation_id),
                &op,
                "Plan military operation",
            )?;
        }
        MilitaryCommand::EstablishOccupation {
            occupation,
            force,
            node,
            ..
        } => {
            let force_state = force_state(staging, force)?;
            if force_state.location != *node || force_state.status == ForceStatus::Routing {
                return Err(err(
                    ErrorCode::InvalidDecision,
                    "force must be present and not routing before occupation",
                ));
            }
            occupation_tick_key(occupation)?;
            ensure_record_capacity::<OccupationStateRecord>(staging, 0)?;
            let occ = OccupationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: occupation.clone(),
                node: node.clone(),
                occupying_force: force.clone(),
                military_control_per_mille: 700,
                garrison_strength: force_state.actual_strength / 3,
                administrative_reach_per_mille: 0,
                security_per_mille: 500,
                fiscal_capacity_per_mille: 0,
                legitimacy_per_mille: 0,
                collaboration_per_mille: 0,
                resistance_per_mille: 500,
                extraction_burden_per_mille: 0,
                integration: IntegrationStage::MilitaryControl,
                policy_revision: 1,
                pending_provider_outcomes: Default::default(),
            };
            staging.create(
                occupation_reference(occupation),
                &occ,
                "Establish military occupation",
            )?;
            schedule_tick(
                staging,
                SimDuration::days(1),
                None,
                Some(occupation.clone()),
                command_operation(command).clone(),
            )?;
        }
        MilitaryCommand::SetOccupationPolicy {
            occupation,
            policy_revision,
            security_per_mille,
            collaboration_per_mille,
            extraction_burden_per_mille,
            ..
        } => update_occupation(staging, occupation, at, |s| {
            if s.policy_revision != *policy_revision {
                return Err(err(
                    ErrorCode::DomainRecordVersionConflict,
                    "occupation policy revision is stale",
                ));
            }
            s.security_per_mille = *security_per_mille;
            s.collaboration_per_mille = *collaboration_per_mille;
            s.extraction_burden_per_mille = *extraction_burden_per_mille;
            s.policy_revision += 1;
            Ok(())
        })?,
        MilitaryCommand::MilitaryAdministrationAction {
            occupation,
            provider_plugin,
            expected_provider_version,
            ..
        } => {
            let record = staging.record(&ledger_reference())?.ok_or_else(|| {
                err(
                    ErrorCode::DomainRecordNotFound,
                    "military ledger is unavailable",
                )
            })?;
            let mut next = record.decode_payload::<MilitaryLedgerRecord>()?;
            // Each pending effect creates one provider outcome record later.
            ensure_record_capacity::<ProviderOutcomeRecord>(staging, next.pending.len())?;
            let key = command_operation(command).clone();
            next.pending.insert(
                key.clone(),
                PendingMilitaryEffect {
                    operation: key.clone(),
                    provider_plugin: provider_plugin.clone(),
                    kind: "administration".to_owned(),
                    expected_source_version: *expected_provider_version,
                    occupation: Some(occupation.clone()),
                    state: PendingEffectState::Pending,
                },
            );
            next.meta = MilitaryRecordMeta::new(record.version + 1, at, &next)?;
            staging.upsert(ledger_reference(), &next, "Queue military provider effect")?;
        }
        MilitaryCommand::AdvanceTick {
            operation,
            occupation,
            ..
        } => advance_tick(staging, context, operation.as_ref(), occupation.as_ref())?,
        MilitaryCommand::PrepareAmbush {
            force,
            node,
            tactic,
            ..
        } => {
            update_force(staging, force, at, |state| {
                state.prepared_ambush = Some(AmbushPreparation {
                    node: node.clone(),
                    tactic: tactic.clone(),
                    concealment_per_mille: 700,
                    prepared_at: at,
                    expires_at: Some(at.checked_add(SimDuration::days(7)).ok_or_else(|| {
                        err(ErrorCode::InvalidDuration, "ambush expiry overflow")
                    })?),
                });
                Ok(())
            })?;
        }
        MilitaryCommand::ExecuteSpecialOperation {
            force,
            operation_id,
            objective,
            target,
            ..
        } => {
            let current = force_state(staging, force)?;
            ensure_record_capacity::<OperationStateRecord>(staging, 0)?;
            let operation = OperationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: operation_id.clone(),
                key: command_operation(command).clone(),
                owner: current.owner,
                objective: objective.clone(),
                kind: "special".to_owned(),
                forces: vec![force.clone()],
                opposing_force: None,
                phase: OperationPhase::Moving,
                from: current.location,
                destination: target.clone(),
                route_digest: digest(&(force, target))?,
                terrain: String::new(),
                weather: String::new(),
                started_at: at,
                due_at: at
                    .checked_add(SimDuration::days(1))
                    .ok_or_else(|| err(ErrorCode::InvalidDuration, "special operation overflow"))?,
                command_delay_minutes: 0,
                supply_line: None,
                exit_condition: "extract".to_owned(),
            };
            operation.validate()?;
            update_force(staging, force, at, |state| {
                state.active_operation = Some(operation_id.clone());
                state.status = ForceStatus::Moving;
                Ok(())
            })?;
            staging.create(
                operation_reference(operation_id),
                &operation,
                "Start special operation",
            )?;
            schedule_tick(
                staging,
                SimDuration::days(1),
                Some(operation_id.clone()),
                None,
                command_operation(command).clone(),
            )?;
        }
        MilitaryCommand::Recon { .. } => {
            let _ = staging.view.random_range_for_operation(
                &military_random_stream(),
                EvidenceRef::Boundary(context.boundary_id),
                "military_command",
                command_operation(command).as_str(),
                RandomOperationTarget::CanonicalKey(command_operation(command).to_string()),
                0,
                1_000,
                "resolve military operation uncertainty",
            )?;
            staging.push(BoundaryDirective::Emit {
                event_type: "canwu.military.transition_applied.v1".to_owned(),
                summary: "Resolve military operation uncertainty".to_owned(),
                affected: Vec::new(),
            });
        }
    }
    if !internal_tick
        && !matches!(
            command,
            MilitaryCommand::MilitaryAdministrationAction { .. }
        )
    {
        record_command_outcome(
            staging,
            command,
            command_digest,
            at,
            OutcomeDisposition::Accepted,
            "Military command applied exactly once",
        )?;
    }
    Ok(())
}

fn require_opposing_force(staging: &Staging<'_, '_>, force: &ForceId) -> Result<(), CanwuError> {
    if staging.record(&force_reference(force))?.is_none() {
        return Err(err(
            ErrorCode::DomainRecordNotFound,
            "opposing force is unavailable",
        ));
    }
    Ok(())
}

fn record_command_outcome(
    staging: &mut Staging<'_, '_>,
    command: &MilitaryCommand,
    input_digest: String,
    at: SimTime,
    disposition: OutcomeDisposition,
    message: &str,
) -> Result<(), CanwuError> {
    let key = command_operation(command).clone();
    let current = staging.record(&ledger_reference())?;
    let mut ledger = current
        .as_ref()
        .map(DomainRecord::decode_payload::<MilitaryLedgerRecord>)
        .transpose()?
        .unwrap_or(MilitaryLedger {
            meta: MilitaryRecordMeta::new(1, at, &())?,
            outcomes: BTreeMap::new(),
            pending: BTreeMap::new(),
        });
    ledger.outcomes.insert(
        key.clone(),
        MilitaryOutcome {
            operation: key,
            input_digest,
            disposition,
            record: "command".to_owned(),
            message: message.to_owned(),
            at,
        },
    );
    ledger.meta.revision = current.as_ref().map_or(1, |record| record.version + 1);
    ledger.meta.established_at = at;
    ledger.meta.semantic_digest = digest(&ledger)?;
    match current {
        Some(_record) => staging.upsert(
            ledger_reference(),
            &ledger,
            "Record military command outcome",
        ),
        None => staging.create(
            ledger_reference(),
            &ledger,
            "Create military command ledger",
        ),
    }
}

fn err(code: ErrorCode, message: impl Into<String>) -> CanwuError {
    CanwuError::new(code, message.into())
}
fn encode(error: serde_json::Error) -> CanwuError {
    err(ErrorCode::InvalidPayload, error.to_string())
}
fn decode<T: DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|e| {
        err(
            ErrorCode::InvalidPayload,
            format!("{label} is invalid: {e}"),
        )
    })
}
fn command_operation(command: &MilitaryCommand) -> &MilitaryOperationKey {
    match command {
        MilitaryCommand::CreateForce { operation, .. }
        | MilitaryCommand::AssignCommander { operation, .. }
        | MilitaryCommand::Recruit { operation, .. }
        | MilitaryCommand::TrainAndEquip { operation, .. }
        | MilitaryCommand::OrderMarch { operation, .. }
        | MilitaryCommand::PlanOperation { operation, .. }
        | MilitaryCommand::Recon { operation, .. }
        | MilitaryCommand::PrepareAmbush { operation, .. }
        | MilitaryCommand::ExecuteSpecialOperation { operation, .. }
        | MilitaryCommand::EstablishOccupation { operation, .. }
        | MilitaryCommand::SetOccupationPolicy { operation, .. }
        | MilitaryCommand::MilitaryAdministrationAction { operation, .. }
        | MilitaryCommand::AdvanceTick {
            operation_key: operation,
            ..
        } => operation,
    }
}
fn command_force(command: &MilitaryCommand) -> Option<&ForceId> {
    match command {
        MilitaryCommand::CreateForce { force, .. }
        | MilitaryCommand::AssignCommander { force, .. }
        | MilitaryCommand::Recruit { force, .. }
        | MilitaryCommand::TrainAndEquip { force, .. }
        | MilitaryCommand::OrderMarch { force, .. }
        | MilitaryCommand::Recon { force, .. }
        | MilitaryCommand::PrepareAmbush { force, .. }
        | MilitaryCommand::ExecuteSpecialOperation { force, .. }
        | MilitaryCommand::EstablishOccupation { force, .. }
        | MilitaryCommand::PlanOperation { force, .. } => Some(force),
        MilitaryCommand::SetOccupationPolicy { .. }
        | MilitaryCommand::MilitaryAdministrationAction { .. }
        | MilitaryCommand::AdvanceTick { .. } => None,
    }
}
fn command_expected_revision(command: &MilitaryCommand) -> Option<u64> {
    match command {
        MilitaryCommand::AssignCommander {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::Recruit {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::TrainAndEquip {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::OrderMarch {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::Recon {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::PrepareAmbush {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::EstablishOccupation {
            expected_force_revision,
            ..
        } => Some(*expected_force_revision),
        _ => None,
    }
}
fn input_digest<T: Serialize>(value: &T) -> Result<String, CanwuError> {
    crate::model::input_digest(value)
}
fn combat_id(operation: &OperationId) -> Result<CombatId, CanwuError> {
    CombatId::new(format!("canwu.military:combat:{operation}"))
}
fn victory_occupation_id(operation: &OperationId) -> Result<OccupationId, CanwuError> {
    OccupationId::new(format!("canwu.military:occupation:{}", operation.as_str()))
}
fn occupation_tick_key(occupation: &OccupationId) -> Result<MilitaryOperationKey, CanwuError> {
    MilitaryOperationKey::new(format!(
        "canwu.military:occupation-tick:{}",
        occupation.as_str()
    ))
}
/// Rejects an operation ID whose derived combat, occupation, or tick IDs
/// would exceed the identifier limit when a later tick builds them.
fn ensure_derived_ids(operation: &OperationId) -> Result<(), CanwuError> {
    combat_id(operation)
        .and_then(|_| victory_occupation_id(operation))
        .and_then(|occupation| occupation_tick_key(&occupation))
        .map(|_| ())
        .map_err(|_| {
            err(
                ErrorCode::InvalidPayload,
                "operation ID is too long for its derived combat and occupation IDs",
            )
        })
}
fn update_force(
    staging: &mut Staging<'_, '_>,
    id: &ForceId,
    at: SimTime,
    change: impl FnOnce(&mut ForceState) -> Result<(), CanwuError>,
) -> Result<(), CanwuError> {
    let reference = force_reference(id);
    let record = staging
        .record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "force is unavailable"))?;
    let mut state = record.decode_payload::<ForceStateRecord>()?;
    change(&mut state)?;
    state.meta.revision = record.version + 1;
    state.meta.established_at = at;
    state.meta.semantic_digest = digest(&state)?;
    state.validate()?;
    staging.upsert(reference, &state, "Update military force")
}
fn update_occupation(
    staging: &mut Staging<'_, '_>,
    id: &OccupationId,
    at: SimTime,
    change: impl FnOnce(&mut OccupationState) -> Result<(), CanwuError>,
) -> Result<(), CanwuError> {
    let reference = occupation_reference(id);
    let record = staging
        .record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "occupation is unavailable"))?;
    let mut state = record.decode_payload::<OccupationStateRecord>()?;
    change(&mut state)?;
    state.meta.revision = record.version + 1;
    state.meta.established_at = at;
    state.meta.semantic_digest = digest(&state)?;
    state.validate()?;
    staging.upsert(reference, &state, "Update military occupation")
}
fn force_state(staging: &Staging<'_, '_>, id: &ForceId) -> Result<ForceState, CanwuError> {
    staging
        .record(&force_reference(id))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "force is unavailable"))?
        .decode_payload::<ForceStateRecord>()
}
fn apply_ack(
    staging: &mut Staging<'_, '_>,
    context: &BoundaryContext,
    outcome: &ProviderOutcome,
) -> Result<(), CanwuError> {
    let reference = ledger_reference();
    let record = staging.record(&reference)?.ok_or_else(|| {
        err(
            ErrorCode::DomainRecordNotFound,
            "military ledger is unavailable",
        )
    })?;
    let mut state = record.decode_payload::<MilitaryLedgerRecord>()?;
    let pending = state.pending.get(&outcome.operation).ok_or_else(|| {
        err(
            ErrorCode::InvalidAuthority,
            "provider outcome has no matching pending military effect",
        )
    })?;
    if pending.provider_plugin != outcome.provider_plugin
        || pending.expected_source_version != outcome.provider_version
    {
        return Err(err(
            ErrorCode::InvalidAuthority,
            "provider outcome identity does not match pending effect",
        ));
    }
    if outcome.digest.is_empty() || outcome.provider_record.is_empty() {
        return Err(err(
            ErrorCode::InvalidPayload,
            "provider outcome lacks an authenticated digest or record",
        ));
    }
    let pending_occupation = pending.occupation.clone();
    state.pending.remove(&outcome.operation);
    state.outcomes.insert(
        outcome.operation.clone(),
        MilitaryOutcome {
            operation: outcome.operation.clone(),
            input_digest: String::new(),
            disposition: match outcome.disposition {
                ProviderDisposition::Rejected => OutcomeDisposition::Rejected,
                _ => OutcomeDisposition::Accepted,
            },
            record: outcome.provider_record.clone(),
            message: "Provider outcome acknowledged".to_owned(),
            at: context.at,
        },
    );
    state.meta.revision = record.version + 1;
    state.meta.established_at = context.at;
    state.meta.semantic_digest = digest(&state)?;
    staging.upsert(reference, &state, "Acknowledge military provider outcome")?;
    if staging
        .record(&provider_outcome_reference(&outcome.id))?
        .is_none()
    {
        staging.create(
            provider_outcome_reference(&outcome.id),
            outcome,
            "Retain exact provider outcome",
        )?;
    }
    if let Some(occupation_id) = pending_occupation {
        let occupation_ref = occupation_reference(&occupation_id);
        if let Some(occupation_record) = staging.record(&occupation_ref)? {
            let mut occupation = occupation_record.decode_payload::<OccupationStateRecord>()?;
            match outcome.disposition {
                ProviderDisposition::Rejected | ProviderDisposition::Compensating => {
                    occupation.integration = IntegrationStage::MilitaryControl;
                    occupation.legitimacy_per_mille =
                        occupation.legitimacy_per_mille.saturating_sub(50);
                }
                ProviderDisposition::Accepted | ProviderDisposition::Committed => {
                    occupation.integration = match occupation.integration {
                        IntegrationStage::MilitaryControl => {
                            IntegrationStage::AdministrativeTakeover
                        }
                        IntegrationStage::AdministrativeTakeover => {
                            IntegrationStage::LegalRecognition
                        }
                        current_stage => current_stage,
                    };
                    occupation
                        .pending_provider_outcomes
                        .insert(outcome.id.clone());
                }
            }
            occupation.meta.revision = occupation_record.version + 1;
            occupation.meta.established_at = context.at;
            occupation.meta.semantic_digest = digest(&occupation)?;
            staging.upsert(
                occupation_ref,
                &occupation,
                "Apply provider result to occupation",
            )?;
        }
    }
    Ok(())
}
fn schedule_tick(
    staging: &mut Staging<'_, '_>,
    after: SimDuration,
    operation: Option<OperationId>,
    occupation: Option<OccupationId>,
    operation_key: MilitaryOperationKey,
) -> Result<(), CanwuError> {
    let command = MilitaryCommand::AdvanceTick {
        operation,
        occupation,
        operation_key: operation_key.clone(),
    };
    let envelope = MilitaryCommandEnvelope {
        input_digest: input_digest(&command)?,
        command,
    };
    staging.push(BoundaryDirective::SchedulePluginIngress {
        target_plugin: PLUGIN_NAME.to_owned(),
        after,
        packet_type: MILITARY_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedCommand { envelope }).map_err(encode)?,
        affected: Vec::new(),
    });
    Ok(())
}

fn advance_tick(
    staging: &mut Staging<'_, '_>,
    context: &BoundaryContext,
    operation: Option<&OperationId>,
    occupation: Option<&OccupationId>,
) -> Result<(), CanwuError> {
    if let Some(operation) = operation {
        advance_operation(staging, context, operation)?;
    }
    if let Some(occupation) = occupation {
        advance_occupation_state(staging, context, occupation)?;
    }
    Ok(())
}

fn advance_operation(
    staging: &mut Staging<'_, '_>,
    context: &BoundaryContext,
    id: &OperationId,
) -> Result<(), CanwuError> {
    let reference = operation_reference(id);
    let record = staging
        .record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "operation is unavailable"))?;
    let mut operation = record.decode_payload::<OperationStateRecord>()?;
    if matches!(
        operation.phase,
        OperationPhase::Completed | OperationPhase::Failed | OperationPhase::Cancelled
    ) {
        return Ok(());
    }
    let force_id = operation
        .forces
        .first()
        .cloned()
        .ok_or_else(|| err(ErrorCode::InvalidDomainRecord, "operation has no force"))?;
    let force_record = staging
        .record(&force_reference(&force_id))?
        .ok_or_else(|| {
            err(
                ErrorCode::DomainRecordNotFound,
                "operation force is unavailable",
            )
        })?;
    let mut force = force_record.decode_payload::<ForceStateRecord>()?;
    let mut force_changed = false;
    if operation.phase == OperationPhase::Moving && context.at >= operation.due_at {
        force_changed = true;
        if force.supply_per_mille < 100 {
            force.status = ForceStatus::Routing;
            force.active_operation = None;
            operation.phase = OperationPhase::Failed;
        } else {
            force.supply_per_mille = force.supply_per_mille.saturating_sub(100);
            force.fatigue_per_mille = force.fatigue_per_mille.saturating_add(20).min(1_000);
            force.morale_per_mille = force.morale_per_mille.saturating_sub(10);
        }
        if operation.phase != OperationPhase::Failed && operation.kind == "special" {
            let success = staging.view.random_range_for_operation(
                &military_random_stream(),
                context
                    .admitted_ingress
                    .first()
                    .copied()
                    .map(EvidenceRef::Ingress)
                    .unwrap_or(EvidenceRef::Boundary(context.boundary_id)),
                "special-operation",
                operation.key.as_str(),
                RandomOperationTarget::CanonicalKey(operation.id.to_string()),
                0,
                1_000,
                "special-operation-success",
            )? >= 650;
            force.location = if success {
                operation.from.clone()
            } else {
                operation.destination.clone()
            };
            force.status = if success {
                ForceStatus::Ready
            } else {
                ForceStatus::Routing
            };
            force.active_operation = None;
            operation.phase = if success {
                OperationPhase::Completed
            } else {
                OperationPhase::Failed
            };
        } else if operation.phase != OperationPhase::Failed {
            force.location = operation.destination.clone();
            force.status = if operation.opposing_force.is_some() {
                ForceStatus::Engaged
            } else {
                ForceStatus::Ready
            };
        }
        // A special operation has already settled above; only an arriving
        // march completes or makes contact here.
        let arrived_march =
            operation.phase != OperationPhase::Failed && operation.kind != "special";
        if arrived_march && operation.opposing_force.is_none() {
            force.active_operation = None;
            operation.phase = OperationPhase::Completed;
        } else if let (true, Some(defender)) = (arrived_march, operation.opposing_force.clone()) {
            operation.phase = OperationPhase::Engaged;
            let defender_record =
                staging
                    .record(&force_reference(&defender))?
                    .ok_or_else(|| {
                        err(
                            ErrorCode::DomainRecordNotFound,
                            "opposing force is unavailable",
                        )
                    })?;
            let mut defender_state = defender_record.decode_payload::<ForceStateRecord>()?;
            let ambush_ready = defender_state
                .prepared_ambush
                .as_ref()
                .is_some_and(|ambush| {
                    ambush.node == operation.destination
                        && ambush
                            .expires_at
                            .is_none_or(|expires| expires >= context.at)
                });
            let combat_id = combat_id(&operation.id)?;
            if staging.record(&combat_reference(&combat_id))?.is_none() {
                let combat = CombatState {
                    meta: MilitaryRecordMeta::new(1, context.at, &())?,
                    id: combat_id.clone(),
                    operation: operation.id.clone(),
                    location: operation.destination.clone(),
                    attacker: force_id.clone(),
                    defender: defender.clone(),
                    stage: CombatStage::Contact,
                    round: 0,
                    attacker_tactic: "screen-and-advance".to_owned(),
                    defender_tactic: "hold".to_owned(),
                    attacker_preparation_per_mille: 500,
                    defender_preparation_per_mille: if ambush_ready { 850 } else { 500 },
                    attacker_visible_strength: force.actual_strength,
                    defender_visible_strength: defender_state.actual_strength,
                    attacker_casualties: 0,
                    defender_casualties: 0,
                    attacker_prisoners: 0,
                    defender_prisoners: 0,
                    result: None,
                    random_envelopes: Vec::new(),
                    causal_notes: vec!["contact confirmed at destination".to_owned()],
                };
                staging.create(
                    combat_reference(&combat_id),
                    &combat,
                    "Create military contact",
                )?;
                if ambush_ready {
                    defender_state.prepared_ambush = None;
                    defender_state.meta.revision = defender_record.version + 1;
                    defender_state.meta.established_at = context.at;
                    defender_state.meta.semantic_digest = digest(&defender_state)?;
                    staging.upsert(
                        force_reference(&defender),
                        &defender_state,
                        "Consume prepared ambush",
                    )?;
                }
            }
            schedule_tick(
                staging,
                SimDuration::days(1),
                Some(operation.id.clone()),
                None,
                operation.key.clone(),
            )?;
        }
    } else if operation.phase == OperationPhase::Engaged {
        resolve_combat_round(staging, context, &mut operation)?;
    }
    if force_changed {
        force.meta.revision = force_record.version + 1;
        force.meta.established_at = context.at;
        force.meta.semantic_digest = digest(&force)?;
        force.validate()?;
        staging.upsert(force_reference(&force_id), &force, "Advance military force")?;
    }
    operation.meta.revision = record.version + 1;
    operation.meta.established_at = context.at;
    operation.meta.semantic_digest = digest(&operation)?;
    staging.upsert(reference, &operation, "Advance military operation")?;
    Ok(())
}

fn resolve_combat_round(
    staging: &mut Staging<'_, '_>,
    context: &BoundaryContext,
    operation: &mut OperationState,
) -> Result<(), CanwuError> {
    let combat_id = combat_id(&operation.id)?;
    let combat_reference = combat_reference(&combat_id);
    let combat_record = staging
        .record(&combat_reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "combat is unavailable"))?;
    let mut combat = combat_record.decode_payload::<CombatStateRecord>()?;
    let attacker_record = staging
        .record(&force_reference(&combat.attacker))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "attacker is unavailable"))?;
    let defender_record = staging
        .record(&force_reference(&combat.defender))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "defender is unavailable"))?;
    let mut attacker = attacker_record.decode_payload::<ForceStateRecord>()?;
    let mut defender = defender_record.decode_payload::<ForceStateRecord>()?;
    let sample = staging.view.random_range_for_operation(
        &military_random_stream(),
        context
            .admitted_ingress
            .first()
            .copied()
            .map(EvidenceRef::Ingress)
            .unwrap_or(EvidenceRef::Boundary(context.boundary_id)),
        "combat-round",
        operation.key.as_str(),
        RandomOperationTarget::DomainRecord {
            record: combat_reference.clone().into_untyped(),
            version: combat_record.version,
        },
        u32::from(combat.round),
        1_000,
        "combat-round-remainder",
    )?;
    let attacker_loss = (defender.actual_strength / 20).max(1) + (sample % 8) as u32;
    let defender_loss = (attacker.actual_strength / 18).max(1) + ((999 - sample) % 8) as u32;
    let attacker_loss = attacker_loss.min(attacker.actual_strength);
    let defender_loss = defender_loss.min(defender.actual_strength);
    remove_losses(&mut attacker, attacker_loss);
    attacker.casualties = attacker.casualties.saturating_add(attacker_loss);
    attacker.morale_per_mille = attacker
        .morale_per_mille
        .saturating_sub(u16::try_from(attacker_loss / 2).unwrap_or(u16::MAX));
    attacker.fatigue_per_mille = attacker.fatigue_per_mille.saturating_add(35).min(1_000);
    remove_losses(&mut defender, defender_loss);
    defender.casualties = defender.casualties.saturating_add(defender_loss);
    defender.morale_per_mille = defender
        .morale_per_mille
        .saturating_sub(u16::try_from(defender_loss / 2).unwrap_or(u16::MAX));
    defender.fatigue_per_mille = defender.fatigue_per_mille.saturating_add(35).min(1_000);
    let terminal =
        attacker.actual_strength == 0 || defender.actual_strength == 0 || combat.round >= 3;
    if terminal {
        combat.result = Some(if defender.actual_strength == 0 {
            CombatResult::AttackerVictory
        } else if attacker.actual_strength == 0 {
            CombatResult::DefenderVictory
        } else {
            CombatResult::MutualDisengagement
        });
        combat.stage = CombatStage::Closed;
        operation.phase = if matches!(combat.result, Some(CombatResult::AttackerVictory)) {
            OperationPhase::Completed
        } else {
            OperationPhase::Withdrawing
        };
        attacker.status = if attacker.actual_strength == 0 {
            ForceStatus::Routing
        } else {
            ForceStatus::Ready
        };
        defender.status = if defender.actual_strength == 0 {
            ForceStatus::Routing
        } else {
            ForceStatus::Ready
        };
        attacker.active_operation = None;
        defender.active_operation = None;
        if matches!(combat.result, Some(CombatResult::AttackerVictory)) {
            let occupation_id = victory_occupation_id(&operation.id)?;
            if staging
                .record(&occupation_reference(&occupation_id))?
                .is_none()
            {
                let occupation = OccupationState {
                    meta: MilitaryRecordMeta::new(1, context.at, &())?,
                    id: occupation_id.clone(),
                    node: operation.destination.clone(),
                    occupying_force: attacker.id.clone(),
                    military_control_per_mille: 700,
                    garrison_strength: attacker.actual_strength / 3,
                    administrative_reach_per_mille: 0,
                    security_per_mille: 500,
                    fiscal_capacity_per_mille: 0,
                    legitimacy_per_mille: 0,
                    collaboration_per_mille: 0,
                    resistance_per_mille: 500,
                    extraction_burden_per_mille: 0,
                    integration: IntegrationStage::MilitaryControl,
                    policy_revision: 1,
                    pending_provider_outcomes: BTreeSet::new(),
                };
                staging.create(
                    occupation_reference(&occupation_id),
                    &occupation,
                    "Establish military control after victory",
                )?;
                schedule_tick(
                    staging,
                    SimDuration::days(1),
                    None,
                    Some(occupation_id),
                    operation.key.clone(),
                )?;
            }
        }
    } else {
        combat.round = combat.round.saturating_add(1);
        combat.stage = CombatStage::RoundResolved;
        schedule_tick(
            staging,
            SimDuration::days(1),
            Some(operation.id.clone()),
            None,
            operation.key.clone(),
        )?;
    }
    combat.attacker_casualties = combat.attacker_casualties.saturating_add(attacker_loss);
    combat.defender_casualties = combat.defender_casualties.saturating_add(defender_loss);
    combat.random_envelopes.push(RandomEnvelope {
        purpose: "combat-round-remainder".to_owned(),
        input_digest: input_digest(&(
            attacker_record.version,
            defender_record.version,
            combat.round,
        ))?,
        ruleset_hash: "synthetic-runtime-v1".to_owned(),
        boundary_id: context.boundary_id.to_string(),
        draw_slot: u32::from(combat.round),
        native_value: sample,
        upper_exclusive: 1_000,
    });
    combat.meta.revision = combat_record.version + 1;
    combat.meta.established_at = context.at;
    combat.meta.semantic_digest = digest(&combat)?;
    staging.upsert(combat_reference, &combat, "Resolve combat round")?;
    attacker.meta.revision = attacker_record.version + 1;
    attacker.meta.established_at = context.at;
    attacker.meta.semantic_digest = digest(&attacker)?;
    defender.meta.revision = defender_record.version + 1;
    defender.meta.established_at = context.at;
    defender.meta.semantic_digest = digest(&defender)?;
    attacker.validate()?;
    defender.validate()?;
    staging.upsert(
        force_reference(&attacker.id),
        &attacker,
        "Apply attacker combat result",
    )?;
    staging.upsert(
        force_reference(&defender.id),
        &defender,
        "Apply defender combat result",
    )?;
    Ok(())
}

/// Takes combat losses from subunits in ID order and removes any subunit that
/// reaches zero, so subunit strengths keep adding up to the force strength.
fn remove_losses(force: &mut ForceState, loss: u32) {
    force.actual_strength -= loss;
    let mut remaining = loss;
    for unit in force.subunits.values_mut() {
        let taken = unit.strength.min(remaining);
        unit.strength -= taken;
        remaining -= taken;
    }
    force.subunits.retain(|_, unit| unit.strength > 0);
}

fn advance_occupation_state(
    staging: &mut Staging<'_, '_>,
    context: &BoundaryContext,
    id: &OccupationId,
) -> Result<(), CanwuError> {
    let reference = occupation_reference(id);
    let record = staging
        .record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "occupation unavailable"))?;
    let mut occupation = record.decode_payload::<OccupationStateRecord>()?;
    occupation.security_per_mille = occupation.security_per_mille.saturating_add(25).min(1_000);
    occupation.administrative_reach_per_mille = occupation
        .administrative_reach_per_mille
        .saturating_add(20)
        .min(1_000);
    occupation.resistance_per_mille = occupation.resistance_per_mille.saturating_sub(10);
    occupation.collaboration_per_mille = occupation
        .collaboration_per_mille
        .saturating_add(10)
        .min(1_000);
    occupation.integration = match occupation.integration {
        IntegrationStage::MilitaryControl
            if occupation.administrative_reach_per_mille >= 300
                && occupation.security_per_mille >= 600 =>
        {
            IntegrationStage::AdministrativeTakeover
        }
        IntegrationStage::AdministrativeTakeover
            if occupation.administrative_reach_per_mille >= 500
                && occupation.security_per_mille >= 650 =>
        {
            IntegrationStage::LegalRecognition
        }
        IntegrationStage::LegalRecognition
            if occupation.administrative_reach_per_mille >= 650
                && occupation.security_per_mille >= 700 =>
        {
            IntegrationStage::FiscalIntegration
        }
        IntegrationStage::FiscalIntegration
            if occupation.administrative_reach_per_mille >= 800
                && occupation.security_per_mille >= 750 =>
        {
            IntegrationStage::SocialIntegration
        }
        IntegrationStage::SocialIntegration
            if occupation.administrative_reach_per_mille >= 900
                && occupation.security_per_mille >= 800 =>
        {
            IntegrationStage::CulturalPractice
        }
        IntegrationStage::CulturalPractice
            if occupation.administrative_reach_per_mille >= 1_000
                && occupation.security_per_mille >= 850 =>
        {
            IntegrationStage::Intergenerational
        }
        stage => stage,
    };
    occupation.meta.revision = record.version + 1;
    occupation.meta.established_at = context.at;
    occupation.meta.semantic_digest = digest(&occupation)?;
    staging.upsert(reference, &occupation, "Advance occupation administration")?;
    if occupation.integration != IntegrationStage::Intergenerational {
        schedule_tick(
            staging,
            SimDuration::days(1),
            None,
            Some(id.clone()),
            occupation_tick_key(id)?,
        )?;
    }
    Ok(())
}

fn materialize_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut reports: BTreeMap<PersonId, Vec<(String, KnowledgeRecordDraft)>> = BTreeMap::new();
    let kind = DomainRecordKind::for_type::<ForceStateRecord>();
    for record in view.domain_records_of_kind(&kind, 256)? {
        let force = record.decode_payload::<ForceStateRecord>()?;
        let Some(commander) = force.commander else {
            continue;
        };
        // A dead person's ledger accepts no knowledge; publishing there would
        // fail every boundary after the commander dies.
        if view
            .person_availability(commander)?
            .is_some_and(|availability| availability.life == LifeState::Dead)
        {
            continue;
        }
        let report = serde_json::json!({
            "force": force.id,
            "location": force.location,
            "strength_low": force.actual_strength.saturating_sub(force.actual_strength / 10),
            "strength_high": force.actual_strength.saturating_add(force.actual_strength / 10),
            "supply_low": force.supply_per_mille.saturating_sub(100),
            "supply_high": force.supply_per_mille.saturating_add(100).min(1_000),
            "observed_at": context.at,
        });
        let source_version = view
            .current_domain_record_version(&record.reference)?
            .ok_or_else(|| {
                err(
                    ErrorCode::DomainRecordNotFound,
                    "force evidence is unavailable",
                )
            })?;
        reports.entry(commander).or_default().push((
            format!("{}:{}", force.id, record.version),
            KnowledgeRecordDraft {
                schema: report_schema_id(),
                subjects: vec![KnowledgeSubject {
                    role: "force".to_owned(),
                    target: KnowledgeSubjectTarget::DomainRecord(record.reference.clone()),
                }],
                payload: report,
                as_of: Some(context.at),
                confidence_per_mille: 900,
                origin: KnowledgeOrigin {
                    method: "military-command-report-v1".to_owned(),
                    evidence: vec![EvidenceRef::DomainRecordVersion(source_version)],
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            },
        ));
    }
    // One batch per commander keeps the pass within the kernel's batch limit;
    // commanders past that limit, in ID order, get no report this boundary.
    let mut directives = Vec::new();
    for (commander, forces) in reports
        .into_iter()
        .take(KnowledgeLimitsV1::CURRENT.batches_per_system_boundary)
    {
        let (sources, records): (Vec<String>, Vec<KnowledgeRecordDraft>) =
            forces.into_iter().unzip();
        directives.push(BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(commander),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!(
                "military-report:{commander}:{}",
                digest(&sources)?
            )),
            records,
            summary: "Publish actor-relative military force report".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}
