// Each integration target intentionally uses a different subset of these helpers.
#![allow(dead_code)]

use canwu_api::{
    DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef,
    DomainReference, DomainReferenceTarget, EntityRef, KnowledgeHolderRef, PersonId, SimTime,
};
use canwu_information::{
    InformationLifecycle, InformationLimitsV1, InformationMutationPlan, InformationRecordSet,
    LifecycleRequest, PLUGIN_NAME,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct DetachedLedger {
    records: BTreeMap<DomainRecordRef, DomainRecord>,
    history: BTreeMap<DomainRecordRef, Vec<DomainRecord>>,
}

impl DetachedLedger {
    pub fn plan_and_apply(
        &mut self,
        request: &LifecycleRequest,
    ) -> Result<InformationMutationPlan, String> {
        let plan = self.plan(request)?;
        self.apply(&plan)?;
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
            .expect("fixture record should exist")
    }

    pub fn history(&self, reference: &DomainRecordRef) -> &[DomainRecord] {
        self.history
            .get(reference)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn apply(&mut self, plan: &InformationMutationPlan) -> Result<(), String> {
        for mutation in &plan.mutations {
            match mutation {
                DomainRecordMutation::Create { record } => {
                    if self.records.contains_key(&record.reference) {
                        return Err(format!("duplicate fixture record {}", record.reference));
                    }
                    let stored = DomainRecord {
                        reference: record.reference.clone(),
                        owner: PLUGIN_NAME.to_owned(),
                        class: DomainRecordClass::Record,
                        version: 1,
                        lifecycle: DomainRecordLifecycle::Active,
                        payload: record.payload.clone(),
                        references: record.references.clone(),
                    };
                    self.records
                        .insert(record.reference.clone(), stored.clone());
                    self.history
                        .entry(record.reference.clone())
                        .or_default()
                        .push(stored);
                }
                DomainRecordMutation::Update {
                    record,
                    expected_version,
                } => {
                    let current = self
                        .records
                        .get_mut(&record.reference)
                        .ok_or_else(|| format!("missing fixture record {}", record.reference))?;
                    if current.version != *expected_version {
                        return Err(format!(
                            "fixture record {} expected version {}, found {}",
                            record.reference, expected_version, current.version
                        ));
                    }
                    current.version += 1;
                    current.payload.clone_from(&record.payload);
                    current.references.clone_from(&record.references);
                    self.history
                        .entry(record.reference.clone())
                        .or_default()
                        .push(current.clone());
                }
                DomainRecordMutation::Retire { .. } | DomainRecordMutation::Delete { .. } => {
                    return Err("detached fixture ledger does not apply retire/delete plans".into());
                }
            }
        }
        Ok(())
    }
}

pub fn minute(value: i64) -> SimTime {
    SimTime::from_minutes(value)
}

pub fn person(id: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(id))
}

pub fn institution(id: &str) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Entity(EntityRef::Domain(DomainRecordRef::new(
        "fixture.information",
        "institution",
        id,
    )))
}

pub fn holder_reference(role: &str, holder: &KnowledgeHolderRef) -> DomainReference {
    DomainReference {
        role: role.to_owned(),
        target: match holder {
            KnowledgeHolderRef::Person(person) => {
                DomainReferenceTarget::Core(EntityRef::Person(*person))
            }
            KnowledgeHolderRef::Entity(EntityRef::Domain(reference)) => {
                DomainReferenceTarget::Domain(reference.clone())
            }
            KnowledgeHolderRef::Entity(entity) => DomainReferenceTarget::Core(entity.clone()),
        },
    }
}

pub fn assert_stable_profile_id(value: &str) {
    assert!(value.starts_with("fixture.information."));
    assert!(value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
    }));
}
