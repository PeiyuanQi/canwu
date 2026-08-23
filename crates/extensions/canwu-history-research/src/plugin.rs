use crate::model::{
    AssessmentCore, AssessmentRecord, HistoricalAssessmentCommand, HistoricalPracticeAssessment,
    HistoricalSourcesAssessment, ProductionArchaeologyAssessment, validate_assessment,
};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, CauseRef, Command, CommandContext, CommandIngress, DomainRecordDraft,
    DomainRecordMutation, DomainRecordMutationPolicy, DomainRecordSchema, DomainReference,
    DomainReferenceSchema, DomainReferenceTarget, DomainReferenceTargetKind, ErrorCode,
    IngressClass, IngressPayload, Issuer, KnowledgeHolderRef, PayloadProperty, PayloadSchema,
    PayloadValueType, PluginActionDescriptor, PluginIngressDescriptor, PluginRegistrar,
    SimDuration, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    SystemDirective, TypedDomainRecordRef, canonical_hash,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const ASSESSMENT_COMMAND: &str = "record_assessment_v1";
pub const ASSESSMENT_INGRESS: &str = "assessment_v1";
const INPUT_HASH_DOMAIN: &str = "canwu.history.assessment-input.v1";
const MAX_ASSESSMENTS_PER_BOUNDARY: usize = 21;
const MAX_ASSESSMENTS_PER_PLUGIN: usize = 1_000;
const CAPACITY_REJECTION_EVENT: &str = "historical_assessment_rejected_capacity_v1";

#[derive(Clone, Copy, Debug, Default)]
pub struct HistoricalSourcesPlugin;

#[derive(Clone, Copy, Debug, Default)]
pub struct HistoricalPracticePlugin;

#[derive(Clone, Copy, Debug, Default)]
pub struct ProductionArchaeologyPlugin;

macro_rules! plugin_impl {
    ($plugin:ty, $record:ty, $command:ident, $apply:ident) => {
        impl SimulationPlugin for $plugin {
            fn name(&self) -> &'static str {
                <$record as AssessmentRecord>::PLUGIN_NAME
            }

            fn version(&self) -> &'static str {
                <$record as AssessmentRecord>::PLUGIN_VERSION
            }

            fn semantic_hash(&self) -> &'static str {
                <$record as AssessmentRecord>::SEMANTIC_HASH
            }

            fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
                register::<$record>(registrar, $command, $apply)
            }
        }
    };
}

plugin_impl!(
    HistoricalSourcesPlugin,
    HistoricalSourcesAssessment,
    sources_command,
    apply_sources
);
plugin_impl!(
    HistoricalPracticePlugin,
    HistoricalPracticeAssessment,
    practice_command,
    apply_practice
);
plugin_impl!(
    ProductionArchaeologyPlugin,
    ProductionArchaeologyAssessment,
    archaeology_command,
    apply_archaeology
);

fn sources_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    command::<HistoricalSourcesAssessment>(view, context, payload)
}

fn practice_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    command::<HistoricalPracticeAssessment>(view, context, payload)
}

fn archaeology_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    command::<ProductionArchaeologyAssessment>(view, context, payload)
}

fn apply_sources(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    apply::<HistoricalSourcesAssessment>(view, context)
}

fn apply_practice(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    apply::<HistoricalPracticeAssessment>(view, context)
}

fn apply_archaeology(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    apply::<ProductionArchaeologyAssessment>(view, context)
}

fn register<T: AssessmentRecord>(
    registrar: &mut PluginRegistrar<'_>,
    command_handler: canwu_api::PluginCommandHandler,
    boundary_handler: canwu_api::BoundarySystemHandler,
) -> Result<(), CanwuError>
where
    T::Payload: Clone + serde::de::DeserializeOwned + Serialize,
{
    let mut schema = DomainRecordSchema::for_type::<T>();
    schema.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
    schema.payload_schema = PayloadSchema::Any;
    let mut subject_targets = technology_and_assessment_kinds()
        .into_iter()
        .map(DomainReferenceTargetKind::Domain)
        .collect::<Vec<_>>();
    subject_targets.push(DomainReferenceTargetKind::Domain(
        canwu_api::DomainRecordKind::for_type::<T>(),
    ));
    subject_targets.sort();
    subject_targets.dedup();
    schema.references = vec![
        DomainReferenceSchema {
            role: "core".to_owned(),
            targets: vec![DomainReferenceTargetKind::AnyEntity],
            required: false,
            multiple: true,
            allow_retired: false,
        },
        DomainReferenceSchema {
            role: "subject".to_owned(),
            targets: subject_targets.clone(),
            required: true,
            multiple: false,
            allow_retired: true,
        },
        DomainReferenceSchema {
            role: "contradicts".to_owned(),
            targets: assessment_kinds()
                .into_iter()
                .map(DomainReferenceTargetKind::Domain)
                .collect(),
            required: false,
            multiple: true,
            allow_retired: true,
        },
        DomainReferenceSchema {
            role: "supersedes".to_owned(),
            targets: assessment_kinds()
                .into_iter()
                .map(DomainReferenceTargetKind::Domain)
                .collect(),
            required: false,
            multiple: true,
            allow_retired: true,
        },
    ];
    registrar.register_record_schema(schema.clone())?;
    registrar.register_command(
        PluginActionDescriptor {
            name: ASSESSMENT_COMMAND.to_owned(),
            description: "Record one authority-checked historical assessment".to_owned(),
            payload_schema: assessment_schema(),
            reads: technology_and_assessment_kinds()
                .into_iter()
                .map(|kind| StateKey::new(kind.namespace, kind.name))
                .chain([
                    StateKey::core_commands(),
                    StateKey::core_events(),
                    StateKey::core_evidence(),
                ])
                .collect(),
            writes: Vec::new(),
        },
        command_handler,
    )?;
    registrar.register_ingress(PluginIngressDescriptor {
        name: ASSESSMENT_INGRESS.to_owned(),
        description: "Apply one admitted historical assessment".to_owned(),
        class: IngressClass::ScheduledSystem,
        payload_schema: assessment_schema(),
    })?;
    let mut contract = BoundarySystemContract::new(
        "historical_assessment_apply_v1",
        BoundaryPhase::DomainDeltaProposal,
        SystemCadence::EventDriven,
    );
    contract.reads = technology_and_assessment_kinds()
        .into_iter()
        .map(|kind| StateKey::new(kind.namespace, kind.name))
        .chain([
            StateKey::core_commands(),
            StateKey::core_events(),
            StateKey::core_ingress(),
            StateKey::core_evidence(),
        ])
        .collect();
    contract.writes = vec![schema.state_key()];
    contract.emits = vec![CAPACITY_REJECTION_EVENT.to_owned()];
    contract.visibility = StateVisibility::SameBoundary;
    registrar.register_boundary_system(contract, boundary_handler)
}

fn command<T: AssessmentRecord>(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError>
where
    T::Payload: Clone + serde::de::DeserializeOwned + Serialize,
{
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "historical assessments require tracked command ingress",
        ));
    }
    let envelope: HistoricalAssessmentCommand<T::Payload> =
        serde_json::from_value(payload.clone()).map_err(|error| decode_error(&error))?;
    validate_identifier(&envelope.id)?;
    if T::core(&envelope.assessment).assessor != envelope.subject {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "assessment subject must be its assessor",
        ));
    }
    require_authority(context, &envelope.subject)?;
    validate_assessment::<T>(&envelope.assessment).map_err(invalid_payload)?;
    if T::core(&envelope.assessment).as_of > context.simulation_time {
        return Err(invalid_payload(
            "historical assessment cannot be dated in the future",
        ));
    }
    validate_assessment_evidence(view, T::core(&envelope.assessment))
        .map_err(|error| invalid_payload(error.message))?;
    let reference = TypedDomainRecordRef::<T>::new(&envelope.id);
    if let Some(existing) = view.typed_domain_record(&reference)? {
        let existing_payload = existing.decode_payload::<T>()?;
        if canonical_hash(INPUT_HASH_DOMAIN, &existing_payload)?
            == canonical_hash(INPUT_HASH_DOMAIN, &envelope.assessment)?
        {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "assessment ID was reused with different input",
        ));
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: ASSESSMENT_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(&envelope).map_err(|error| encode_error(&error))?,
        affected: holder_entities(&envelope.subject),
    }])
}

#[allow(clippy::too_many_lines)]
fn apply<T: AssessmentRecord>(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError>
where
    T::Payload: Clone + serde::de::DeserializeOwned + Serialize,
{
    let mut admitted = BTreeMap::<String, (String, T::Payload)>::new();
    let mut conflicts = BTreeSet::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
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
        if plugin != T::PLUGIN_NAME || packet_type != ASSESSMENT_INGRESS {
            continue;
        }
        let envelope: HistoricalAssessmentCommand<T::Payload> =
            serde_json::from_value(payload.clone()).map_err(|error| decode_error(&error))?;
        let hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope.assessment)?;
        let Some(CauseRef::Command(command_id)) = ingress.cause else {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "historical assessment ingress must be caused by an admitted command",
            ));
        };
        let command = view.command(command_id)?.ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "historical assessment command evidence is unavailable",
            )
        })?;
        let command_matches = match &command.envelope.command {
            Command::Plugin {
                plugin,
                command,
                payload,
            } if plugin == T::PLUGIN_NAME && command == ASSESSMENT_COMMAND => {
                serde_json::from_value::<HistoricalAssessmentCommand<T::Payload>>(payload.clone())
                    .is_ok_and(|value| {
                        value.id == envelope.id
                            && value.subject == envelope.subject
                            && canonical_hash(INPUT_HASH_DOMAIN, &value.assessment)
                                .is_ok_and(|value| value == hash)
                    })
            }
            _ => false,
        };
        if !command_matches || T::core(&envelope.assessment).assessor != envelope.subject {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "historical assessment ingress does not match its authorized command",
            ));
        }
        if let Some((prior, _)) = admitted.get(&envelope.id) {
            if prior != &hash {
                conflicts.insert(envelope.id.clone());
            }
            continue;
        }
        admitted.insert(envelope.id, (hash, envelope.assessment));
    }
    let existing_count = count_records::<T>(view)?;
    let remaining_capacity = MAX_ASSESSMENTS_PER_PLUGIN.saturating_sub(existing_count);
    let mut directives = Vec::new();
    let mut created = 0usize;
    for (id, (hash, payload)) in admitted {
        if conflicts.contains(&id) {
            directives.push(capacity_rejection::<T>(
                &id,
                &payload,
                "the assessment ID had conflicting canonical inputs",
            ));
            continue;
        }
        validate_assessment::<T>(&payload).map_err(invalid)?;
        let core = T::core(&payload);
        if core.as_of > context.at {
            return Err(invalid(
                "historical assessment cannot be dated in the future",
            ));
        }
        validate_assessment_evidence(view, core)?;
        let reference = TypedDomainRecordRef::<T>::new(&id);
        if let Some(existing) = view.typed_domain_record(&reference)? {
            let existing = existing.decode_payload::<T>()?;
            if canonical_hash(INPUT_HASH_DOMAIN, &existing)? != hash {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "assessment ID was reused with different input",
                ));
            }
            continue;
        }
        if created >= MAX_ASSESSMENTS_PER_BOUNDARY || created >= remaining_capacity {
            directives.push(capacity_rejection::<T>(
                &id,
                &payload,
                if created >= remaining_capacity {
                    "the historical assessment plugin reached its retained-record limit"
                } else {
                    "the historical assessment boundary reached its mutation budget"
                },
            ));
            continue;
        }
        let mut draft = DomainRecordDraft::from_typed(reference, &payload)?;
        draft.references.push(DomainReference {
            role: "subject".to_owned(),
            target: DomainReferenceTarget::Domain(core.subject.record.clone()),
        });
        draft
            .references
            .extend(core.contradicts.iter().map(|value| DomainReference {
                role: "contradicts".to_owned(),
                target: DomainReferenceTarget::Domain(value.record.clone()),
            }));
        draft
            .references
            .extend(core.supersedes.iter().map(|value| DomainReference {
                role: "supersedes".to_owned(),
                target: DomainReferenceTarget::Domain(value.record.clone()),
            }));
        draft.references.extend(
            holder_entities(&core.assessor)
                .into_iter()
                .chain(T::core_entities(&payload))
                .map(|value| DomainReference {
                    role: "core".to_owned(),
                    target: DomainReferenceTarget::Core(value),
                }),
        );
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record: draft },
            summary: "Record bounded historical evidence assessment".to_owned(),
        });
        created += 1;
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn capacity_rejection<T: AssessmentRecord>(
    id: &str,
    payload: &T::Payload,
    reason: &str,
) -> BoundaryDirective
where
    T::Payload: Clone + serde::de::DeserializeOwned + Serialize,
{
    BoundaryDirective::Emit {
        event_type: CAPACITY_REJECTION_EVENT.to_owned(),
        summary: format!("Reject historical assessment {id}: {reason}"),
        affected: holder_entities(&T::core(payload).assessor),
    }
}

fn count_records<T: AssessmentRecord>(view: &SimulationView<'_>) -> Result<usize, CanwuError>
where
    T::Payload: Clone + serde::de::DeserializeOwned + Serialize,
{
    let kind = canwu_api::DomainRecordKind::for_type::<T>();
    let mut after = None;
    let mut count = 0usize;
    loop {
        let page = view.domain_records_of_kind_after(&kind, after.as_ref(), 256)?;
        if page.is_empty() {
            return Ok(count);
        }
        count = count
            .checked_add(page.len())
            .ok_or_else(|| invalid("historical assessment count overflow"))?;
        after = page.last().map(|record| record.reference.clone());
        if page.len() < 256 {
            return Ok(count);
        }
    }
}

fn technology_and_assessment_kinds() -> Vec<canwu_api::DomainRecordKind> {
    let mut kinds = canwu_technology::technology_record_schemas()
        .into_iter()
        .map(|schema| schema.kind)
        .collect::<Vec<_>>();
    kinds.extend(assessment_kinds());
    kinds
}

fn assessment_kinds() -> Vec<canwu_api::DomainRecordKind> {
    vec![
        canwu_api::DomainRecordKind::for_type::<HistoricalSourcesAssessment>(),
        canwu_api::DomainRecordKind::for_type::<HistoricalPracticeAssessment>(),
        canwu_api::DomainRecordKind::for_type::<ProductionArchaeologyAssessment>(),
    ]
}

fn validate_assessment_evidence(
    view: &SimulationView<'_>,
    core: &AssessmentCore,
) -> Result<(), CanwuError> {
    for reference in std::iter::once(&core.subject)
        .chain(core.contradicts.iter())
        .chain(core.supersedes.iter())
    {
        if !view.domain_record_version_evidence_exists(reference)? {
            return Err(invalid(
                "historical assessment cites unavailable exact evidence",
            ));
        }
        let evidence = canwu_api::EvidenceRef::DomainRecordVersion(reference.clone());
        if view
            .evidence_time(&evidence)?
            .is_none_or(|at| at > core.as_of)
        {
            return Err(invalid(
                "historical assessment cites exact evidence established after its as-of cut",
            ));
        }
    }
    for citation in &core.citations {
        if matches!(citation, canwu_api::EvidenceRef::Ingress(_)) {
            return Err(invalid(
                "historical assessment commands must cite durable evidence rather than transient ingress",
            ));
        }
        if !view.evidence_exists(citation)? {
            return Err(invalid("historical assessment cites unavailable evidence"));
        }
        if view
            .evidence_time(citation)?
            .is_none_or(|at| at > core.as_of)
        {
            return Err(invalid(
                "historical assessment cites evidence established after its as-of cut",
            ));
        }
    }
    for relation in core.contradicts.iter().chain(&core.supersedes) {
        let related_subject = assessment_subject(view, relation)?.ok_or_else(|| {
            invalid("historical contradiction or supersession must cite an assessment record")
        })?;
        if related_subject != core.subject {
            return Err(invalid(
                "historical contradiction or supersession must concern the same exact subject",
            ));
        }
    }
    Ok(())
}

fn assessment_subject(
    view: &SimulationView<'_>,
    reference: &canwu_api::DomainRecordVersionRef,
) -> Result<Option<canwu_api::DomainRecordVersionRef>, CanwuError> {
    let Some(record) = view.domain_record_version(reference)? else {
        return Ok(None);
    };
    let subject = if record
        .reference
        .kind
        .matches_type::<HistoricalSourcesAssessment>()
    {
        record
            .decode_payload::<HistoricalSourcesAssessment>()?
            .core
            .subject
    } else if record
        .reference
        .kind
        .matches_type::<HistoricalPracticeAssessment>()
    {
        record
            .decode_payload::<HistoricalPracticeAssessment>()?
            .core
            .subject
    } else if record
        .reference
        .kind
        .matches_type::<ProductionArchaeologyAssessment>()
    {
        record
            .decode_payload::<ProductionArchaeologyAssessment>()?
            .core
            .subject
    } else {
        return Ok(None);
    };
    Ok(Some(subject))
}

fn assessment_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "assessment".to_owned(),
                PayloadProperty {
                    value_type: PayloadValueType::Object,
                    required: true,
                },
            ),
            (
                "id".to_owned(),
                PayloadProperty {
                    value_type: PayloadValueType::String,
                    required: true,
                },
            ),
            (
                "subject".to_owned(),
                PayloadProperty {
                    value_type: PayloadValueType::Object,
                    required: true,
                },
            ),
        ]),
        allow_additional: false,
    }
}

fn require_authority(
    context: &CommandContext,
    holder: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let authorized = match holder {
        KnowledgeHolderRef::Person(person) => context.issuer == Issuer::Actor(*person),
        KnowledgeHolderRef::Entity(entity) => {
            context.authority.command_subject.as_ref() == Some(entity)
                && context.decision_controller_id.is_some()
        }
    };
    authorized.then_some(()).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "assessment issuer is not authorized for its subject",
        )
    })
}

fn holder_entities(holder: &KnowledgeHolderRef) -> Vec<canwu_api::EntityRef> {
    match holder {
        KnowledgeHolderRef::Person(person) => vec![canwu_api::EntityRef::Person(*person)],
        KnowledgeHolderRef::Entity(entity) => vec![entity.clone()],
    }
}

fn validate_identifier(value: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "assessment identity is not canonical",
        ));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn invalid_payload(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidPayload, message)
}

fn decode_error(error: &serde_json::Error) -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidPayload,
        format!("historical assessment could not be decoded: {error}"),
    )
}

fn encode_error(error: &serde_json::Error) -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidPayload,
        format!("historical assessment could not be encoded: {error}"),
    )
}

pub struct HistoricalResearchSuite;

impl HistoricalResearchSuite {
    #[must_use]
    pub fn plugins() -> [&'static dyn SimulationPlugin; 3] {
        static SOURCES: HistoricalSourcesPlugin = HistoricalSourcesPlugin;
        static PRACTICE: HistoricalPracticePlugin = HistoricalPracticePlugin;
        static ARCHAEOLOGY: ProductionArchaeologyPlugin = ProductionArchaeologyPlugin;
        [&SOURCES, &PRACTICE, &ARCHAEOLOGY]
    }
}
