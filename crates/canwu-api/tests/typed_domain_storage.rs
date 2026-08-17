use canwu_api::{
    Canwu, CanwuError, DomainEntityKindClass, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordSchema, DomainRecordType, DomainValueKindClass, ErrorCode,
    KnowledgeSnapshot, PayloadProperty, PayloadSchema, PayloadValueType, PluginRegistrar, Scenario,
    SimTime, SimulationPlugin, TypedDomainRecordRef, WorldSnapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct OfficePayload {
    title: String,
    capacity: u32,
}

struct Office;

impl DomainRecordType for Office {
    type Payload = OfficePayload;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "fixture.governance";
    const NAME: &'static str = "office";
}

struct Obligation;

impl DomainRecordType for Obligation {
    type Payload = OfficePayload;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.governance";
    const NAME: &'static str = "obligation";
}

struct GovernancePlugin;

impl SimulationPlugin for GovernancePlugin {
    fn name(&self) -> &'static str {
        "fixture-governance"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000a1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_entity::<Office>();
        schema.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "capacity".to_owned(),
                    PayloadProperty {
                        value_type: PayloadValueType::Integer,
                        required: true,
                    },
                ),
                (
                    "title".to_owned(),
                    PayloadProperty {
                        value_type: PayloadValueType::String,
                        required: true,
                    },
                ),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(schema)
    }
}

#[test]
fn typed_domain_storage_is_binding_safe_across_save_and_load() {
    let office = TypedDomainRecordRef::<Office>::new("secretariat");
    let payload = OfficePayload {
        title: "Council Office".to_owned(),
        capacity: 12,
    };
    let draft = DomainRecordDraft::from_typed(office.clone(), &payload)
        .expect("typed payload should encode through the public API");
    let scenario = Scenario {
        start_time: SimTime::EPOCH,
        world: WorldSnapshot::default(),
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![DomainRecord {
            reference: draft.reference,
            owner: "fixture-governance".to_owned(),
            class: DomainRecordClass::Entity,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        }],
    };
    let plugin = GovernancePlugin;
    let canwu = Canwu::new_with_plugins(7, scenario, &[&plugin])
        .expect("typed initial domain state should validate");

    let record = canwu
        .typed_domain_record(&office)
        .expect("typed identity should resolve the stored record");
    assert_eq!(
        record
            .decode_payload::<Office>()
            .expect("stored payload should decode through its declared type"),
        payload
    );
    assert_eq!(record.typed_reference::<Office>(), Some(office.clone()));
    let mismatch = record
        .decode_payload::<Obligation>()
        .expect_err("a different namespaced kind should be rejected before payload decoding");
    assert_eq!(mismatch.code, ErrorCode::InvalidDomainRecord);

    let snapshot = canwu
        .snapshot_json()
        .expect("typed domain state should serialize through the stable snapshot format");
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&plugin])
        .expect("typed domain state should rehydrate with its package");
    assert_eq!(
        restored
            .typed_domain_record(&office)
            .expect("restored typed identity should resolve")
            .decode_payload::<Office>()
            .expect("restored typed payload should decode"),
        payload
    );
}
