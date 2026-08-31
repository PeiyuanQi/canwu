use crate::{
    EconomyPriceProviderPayloadV1, EconomyPriceProviderRecord, EconomyRouteProviderPayloadV1,
    EconomyRouteProviderRecord, economy_price_provider_reference,
    economy_route_provider_reference,
};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, Command, CommandContext, CommandIngress, DecisionOrigin, DomainRecord,
    DomainRecordDraft, DomainRecordMutation, DomainRecordSchema, ErrorCode, IngressClass,
    IngressPayload, Issuer, KnowledgeHolderRef, PayloadSchema, PluginActionDescriptor,
    PluginIngressDescriptor, PluginRegistrar, SimDuration, SimulationPlugin, SimulationView,
    StateKey, StateVisibility, SystemCadence, SystemDirective,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME: &str = "canwu-economy-evidence-adapter";
pub const ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAMESPACE: &str = "canwu.economy-evidence-adapter";
pub const ECONOMY_EVIDENCE_COMMAND: &str = "publish_typed_economy_evidence_v1";
const ECONOMY_EVIDENCE_INGRESS: &str = "typed_economy_evidence_v1";
pub const ECONOMY_EVIDENCE_SEMANTIC_HASH: &str =
    "4aec5a3870c2c929cc31a20f24ff1ef4b70db1f3a2a12867b788b66f9b230674";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "evidence", content = "payload", rename_all = "snake_case")]
pub enum EconomyEvidenceOperationV1 {
    Route(EconomyRouteProviderPayloadV1),
    Price(EconomyPriceProviderPayloadV1),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyEvidenceCommandV1 {
    pub holder: KnowledgeHolderRef,
    pub operation: EconomyEvidenceOperationV1,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EconomyEvidenceAdapterPlugin;

pub fn economy_evidence_command(
    value: &EconomyEvidenceCommandV1,
) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME.to_owned(),
        command: ECONOMY_EVIDENCE_COMMAND.to_owned(),
        payload: serde_json::to_value(value)?,
    })
}

impl SimulationPlugin for EconomyEvidenceAdapterPlugin {
    fn name(&self) -> &'static str {
        ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        ECONOMY_EVIDENCE_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        for record in records {
            if record.reference.kind.matches_type::<EconomyRouteProviderRecord>() {
                let payload = record.decode_payload::<EconomyRouteProviderRecord>()?;
                if record.owner != ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME
                    || payload.provider_plugin != record.owner
                    || record.reference
                        != economy_route_provider_reference(&payload.id).into_untyped()
                    || payload.clone().seal()? != payload
                {
                    return Err(invalid("typed route evidence record is forged"));
                }
            } else if record.reference.kind.matches_type::<EconomyPriceProviderRecord>() {
                let payload = record.decode_payload::<EconomyPriceProviderRecord>()?;
                if record.owner != ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME
                    || payload.provider_plugin != record.owner
                    || record.reference
                        != economy_price_provider_reference(&payload.id).into_untyped()
                    || payload.clone().seal()? != payload
                {
                    return Err(invalid("typed price evidence record is forged"));
                }
            }
        }
        Ok(())
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            EconomyRouteProviderRecord,
        >())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            EconomyPriceProviderRecord,
        >())?;
        registrar.register_command(
            PluginActionDescriptor {
                name: ECONOMY_EVIDENCE_COMMAND.to_owned(),
                description: "Publish one provider-owned typed route or price observation"
                    .to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            admit_evidence,
        )?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: ECONOMY_EVIDENCE_INGRESS.to_owned(),
            description: "Materialize one provider-owned typed economy observation".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut apply = BoundarySystemContract::new(
            "apply-typed-economy-evidence-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = vec![StateKey::core_ingress(), StateKey::new("canwu.resource", "runtime")];
        apply.writes = vec![
            StateKey::new(crate::PLUGIN_NAMESPACE, "route-observation-provider"),
            StateKey::new(crate::PLUGIN_NAMESPACE, "price-observation-provider"),
        ];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, apply_evidence)
    }
}

fn admit_evidence(
    _view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "typed economy evidence requires tracked command ingress",
        ));
    }
    let command: EconomyEvidenceCommandV1 = serde_json::from_value(payload.clone())
        .map_err(|error| invalid(format!("typed economy evidence could not be decoded: {error}")))?;
    require_holder_authority(context, &command.holder)?;
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: ECONOMY_EVIDENCE_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(command)
            .map_err(|error| invalid(format!("typed economy evidence could not be encoded: {error}")))?,
        affected: Vec::new(),
    }])
}

fn apply_evidence(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
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
        if plugin != ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME
            || packet_type != ECONOMY_EVIDENCE_INGRESS
        {
            continue;
        }
        let command: EconomyEvidenceCommandV1 = serde_json::from_value(payload.clone())
            .map_err(|error| invalid(format!("typed economy evidence could not be decoded: {error}")))?;
        let (reference, draft) = match command.operation {
            EconomyEvidenceOperationV1::Route(payload) => {
                validate_route_payload(view, &command.holder, &payload)?;
                let sealed = payload.seal()?;
                let reference = economy_route_provider_reference(&sealed.id);
                let untyped = reference.clone().into_untyped();
                (untyped, DomainRecordDraft::from_typed(reference, &sealed)?)
            }
            EconomyEvidenceOperationV1::Price(payload) => {
                validate_price_payload(view, &command.holder, &payload)?;
                let sealed = payload.seal()?;
                let reference = economy_price_provider_reference(&sealed.id);
                let untyped = reference.clone().into_untyped();
                (untyped, DomainRecordDraft::from_typed(reference, &sealed)?)
            }
        };
        if view.domain_record(&reference)?.is_some() {
            return Err(CanwuError::new(
                ErrorCode::DuplicateDomainRecord,
                "typed economy evidence identity already exists",
            ));
        }
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record: draft },
            summary: "publish provider-owned typed economy evidence".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn validate_route_payload(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
    payload: &EconomyRouteProviderPayloadV1,
) -> Result<(), CanwuError> {
    if &payload.holder != holder
        || payload.provider_plugin != ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME
        || payload.route_key.is_empty()
        || payload.confidence_per_mille > 1_000
        || payload.source_versions.is_empty()
    {
        return Err(invalid("typed route evidence envelope is invalid"));
    }
    validate_sources(view, &payload.source_versions)
}

fn validate_price_payload(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
    payload: &EconomyPriceProviderPayloadV1,
) -> Result<(), CanwuError> {
    if &payload.holder != holder
        || payload.provider_plugin != ECONOMY_EVIDENCE_ADAPTER_PLUGIN_NAME
        || payload.scale == 0
        || payload.confidence_per_mille > 1_000
        || payload.source_versions.is_empty()
    {
        return Err(invalid("typed price evidence envelope is invalid"));
    }
    validate_sources(view, &payload.source_versions)
}

fn validate_sources(
    view: &SimulationView<'_>,
    sources: &[canwu_api::DomainRecordVersionRef],
) -> Result<(), CanwuError> {
    for source in sources {
        let record = view
            .domain_record_version(source)?
            .ok_or_else(|| invalid("typed economy evidence source is unavailable"))?;
        if record.reference != canwu_resource::resource_runtime_reference().into_untyped()
            || record.owner != canwu_resource::PLUGIN_NAME
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "typed economy evidence source is not provider-authentic",
            ));
        }
    }
    Ok(())
}

fn require_holder_authority(
    context: &CommandContext,
    holder: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let authorized = match holder {
        KnowledgeHolderRef::Person(person) => {
            let origin_matches =
                context.authority.decision_origin == DecisionOrigin::Actor { actor: *person };
            let issuer_matches = match &context.issuer {
                Issuer::Actor(actor) => actor == person,
                Issuer::Human(controller) | Issuer::Ai(controller) => {
                    context.decision_controller_id.as_deref() == Some(controller)
                }
                _ => false,
            };
            origin_matches && issuer_matches
        }
        KnowledgeHolderRef::Entity(entity) => {
            context.authority.command_subject.as_ref() == Some(entity)
                && context.decision_controller_id.is_some()
        }
    };
    if authorized {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "typed economy evidence issuer is not authorized for its holder",
        ))
    }
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
