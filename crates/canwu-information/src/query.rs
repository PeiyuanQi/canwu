use crate::schema::{DeliveryAttempt, Dispatch};
use canwu_api::{
    DomainRecord, DomainRecordKind, DomainRecordRef, DomainRecordType, DomainReferenceTarget,
    KnowledgeHolderRef, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Deterministic query selectors over detached authoritative information records.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct InformationQuery {
    pub kinds: Vec<DomainRecordKind>,
    pub active_only: bool,
    pub reference_role: Option<String>,
    pub reference_target: Option<DomainReferenceTarget>,
}

/// Detached record index used by the pure lifecycle planner.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct InformationRecordSet {
    records: BTreeMap<DomainRecordRef, DomainRecord>,
}

impl InformationRecordSet {
    pub fn from_records(records: impl IntoIterator<Item = DomainRecord>) -> Result<Self, String> {
        let mut indexed = BTreeMap::new();
        for record in records {
            if indexed.insert(record.reference.clone(), record).is_some() {
                return Err("information record set contains a duplicate reference".to_owned());
            }
        }
        Ok(Self { records: indexed })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub fn record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.records.get(reference)
    }

    pub fn required<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Result<&DomainRecord, String> {
        self.record(reference.as_untyped()).ok_or_else(|| {
            format!("information record {reference} is not available at the validation cut")
        })
    }

    pub fn decode<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Result<T::Payload, String>
    where
        T::Payload: serde::de::DeserializeOwned,
    {
        self.required(reference)?
            .decode_payload::<T>()
            .map_err(|error| error.to_string())
    }

    pub fn query(&self, query: &InformationQuery) -> Result<Vec<&DomainRecord>, String> {
        let mut kinds = query.kinds.clone();
        kinds.sort();
        kinds.dedup();
        if query.reference_role.is_some() != query.reference_target.is_some() {
            return Err(
                "reference-role and reference-target query filters must be supplied together"
                    .to_owned(),
            );
        }
        Ok(self
            .records
            .values()
            .filter(|record| {
                kinds.is_empty() || kinds.binary_search(&record.reference.kind).is_ok()
            })
            .filter(|record| !query.active_only || record.is_active())
            .filter(
                |record| match (&query.reference_role, &query.reference_target) {
                    (Some(role), Some(target)) => record
                        .references
                        .iter()
                        .any(|reference| &reference.role == role && &reference.target == target),
                    (None, None) => true,
                    (Some(_), None) | (None, Some(_)) => false,
                },
            )
            .collect())
    }

    pub fn delivery_attempts(
        &self,
        dispatch: &TypedDomainRecordRef<Dispatch>,
        recipient: Option<&KnowledgeHolderRef>,
    ) -> Result<Vec<&DomainRecord>, String> {
        let dispatch_target = DomainReferenceTarget::Domain(dispatch.as_untyped().clone());
        let recipient_target = recipient.map(holder_target);
        let mut attempts: Vec<_> = self
            .records
            .values()
            .filter(|record| record.reference.kind.matches_type::<DeliveryAttempt>())
            .filter(|record| {
                has_reference(record, "dispatch", &dispatch_target)
                    && recipient_target
                        .as_ref()
                        .is_none_or(|target| has_reference(record, "recipient", target))
            })
            .collect();
        attempts.sort_by_key(|record| {
            record
                .decode_payload::<DeliveryAttempt>()
                .map_or(u32::MAX, |payload| payload.attempt_number)
        });
        for (index, record) in attempts.iter().enumerate() {
            let payload = record
                .decode_payload::<DeliveryAttempt>()
                .map_err(|error| error.to_string())?;
            if payload.attempt_number as usize != index + 1 {
                return Err(
                    "delivery-attempt query found a non-contiguous attempt sequence".to_owned(),
                );
            }
        }
        Ok(attempts)
    }
}

fn has_reference(record: &DomainRecord, role: &str, target: &DomainReferenceTarget) -> bool {
    record
        .references
        .iter()
        .any(|reference| reference.role == role && &reference.target == target)
}

fn holder_target(holder: &KnowledgeHolderRef) -> DomainReferenceTarget {
    match holder {
        KnowledgeHolderRef::Person(person) => {
            DomainReferenceTarget::Core(canwu_api::EntityRef::Person(*person))
        }
        KnowledgeHolderRef::Entity(canwu_api::EntityRef::Domain(reference)) => {
            DomainReferenceTarget::Domain(reference.clone())
        }
        KnowledgeHolderRef::Entity(entity) => DomainReferenceTarget::Core(entity.clone()),
    }
}
