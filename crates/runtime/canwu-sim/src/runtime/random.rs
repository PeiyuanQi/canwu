use super::{CanwuError, ErrorCode};
use canwu_core::{
    ArmyId, BoundaryId, DeterministicRng, DomainRecordRef, EntityRef, EventId, EvidenceRef,
    KnowledgeHolderRef, PersonId, RandomDrawId,
};
use canwu_event::CauseRef;
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const STREAM_DERIVATION_DOMAIN: &[u8] = b"canwu.random-stream.v1";
const OPERATION_DOMAIN: &[u8] = b"canwu.random.operation.v1";
const PURPOSE_DOMAIN: &[u8] = b"canwu.random.purpose.v1";
const OPERATION_TEXT_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RandomAlgorithm {
    /// Current unbiased `SplitMix64` range reduction.
    #[default]
    SplitMix64V2,
    /// Historical modulo-reduction behavior retained for existing journals.
    SplitMix64V1,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RandomStreamKey {
    pub namespace: String,
    pub name: String,
    pub version: u32,
}

impl RandomStreamKey {
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, version: u32) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            version,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RandomStreamState {
    pub key: RandomStreamKey,
    pub algorithm: RandomAlgorithm,
    pub seed: u64,
    pub position: u64,
    pub generator_state: u64,
}

impl RandomStreamState {
    pub(crate) fn initial(root_seed: u64, key: RandomStreamKey) -> Self {
        let seed = derive_stream_seed(root_seed, &key);
        Self {
            key,
            algorithm: RandomAlgorithm::SplitMix64V2,
            seed,
            position: 0,
            generator_state: seed,
        }
    }

    pub(crate) fn is_coherent(&self, root_seed: u64) -> bool {
        matches!(
            self.algorithm,
            RandomAlgorithm::SplitMix64V1 | RandomAlgorithm::SplitMix64V2
        ) && self.seed == derive_stream_seed(root_seed, &self.key)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RandomDrawProducer {
    BoundarySystem {
        boundary: BoundaryId,
        plugin: String,
        system: String,
    },
    CoreSystem {
        system: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RandomDrawOutcome {
    BoundarySystemDecision,
    KnowledgeReportDelivery {
        recipient: PersonId,
        army: ArmyId,
        dispatch_event: EventId,
        arrives_at: SimTime,
    },
}

/// Stable application target for an operation-addressed random draw.
///
/// Format-7 target for the enabled byte-exact keyed algorithm.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RandomOperationTarget {
    Entity(EntityRef),
    DomainRecord {
        record: DomainRecordRef,
        version: u64,
    },
    KnowledgeHolder(KnowledgeHolderRef),
    CanonicalKey(String),
}

/// Version-one stable entropy address for a future keyed random draw.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RandomOperationAddressV1 {
    pub producer_plugin: String,
    pub operation_kind: String,
    pub application_operation_id: String,
    pub target: RandomOperationTarget,
    pub draw_slot: u32,
}

/// Persisted address of a random draw.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RandomDrawAddress {
    Sequential { position: u64 },
    OperationV1(RandomOperationAddressV1),
}

impl RandomDrawAddress {
    #[must_use]
    pub const fn sequential_position(&self) -> Option<u64> {
        match self {
            Self::Sequential { position } => Some(*position),
            Self::OperationV1(_) => None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RandomDrawRecord {
    pub id: RandomDrawId,
    pub at: SimTime,
    pub stream: RandomStreamKey,
    pub address: RandomDrawAddress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_evidence: Option<EvidenceRef>,
    pub upper_exclusive: u64,
    pub value: u64,
    pub purpose: String,
    pub producer: RandomDrawProducer,
    #[serde(default)]
    pub outcome: Option<RandomDrawOutcome>,
    pub cause: CauseRef,
    pub correlation_id: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeyedDrawReservation {
    pub stream: RandomStreamKey,
    pub address: RandomOperationAddressV1,
    pub upper_exclusive: u64,
    pub purpose_hash: String,
    pub result: u64,
    pub draw_id: RandomDrawId,
    pub operation_evidence: EvidenceRef,
    pub draw_receipt: crate::ArchivedEvidenceReceipt,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingRandomDraw {
    pub stream: RandomStreamKey,
    pub address: RandomDrawAddress,
    pub operation_evidence: Option<EvidenceRef>,
    pub upper_exclusive: u64,
    pub value: u64,
    pub purpose: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeyedDrawMemo {
    pub operation_evidence: EvidenceRef,
    pub upper_exclusive: u64,
    pub value: u64,
    pub purpose_hash: String,
}

pub(crate) type KeyedDrawIndex =
    BTreeMap<(RandomStreamKey, RandomOperationAddressV1), KeyedDrawMemo>;

pub(crate) struct RandomExecution {
    pub states: BTreeMap<RandomStreamKey, RandomStreamState>,
    pub draws: Vec<PendingRandomDraw>,
}

pub(crate) struct RandomSession {
    states: BTreeMap<RandomStreamKey, RandomStreamState>,
    draws: Vec<PendingRandomDraw>,
    root_seed: u64,
    producer_plugin: String,
    keyed: KeyedDrawIndex,
}

impl RandomSession {
    pub(crate) fn new(
        available: &BTreeMap<RandomStreamKey, RandomStreamState>,
        allowed: &[RandomStreamKey],
        root_seed: u64,
        producer_plugin: &str,
        keyed: &KeyedDrawIndex,
    ) -> Result<Self, CanwuError> {
        let mut states = BTreeMap::new();
        for key in allowed {
            let Some(state) = available.get(key) else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidRandomStream,
                    format!(
                        "declared random stream {}.{}@{} is not initialized",
                        key.namespace, key.name, key.version
                    ),
                ));
            };
            states.insert(key.clone(), state.clone());
        }
        Ok(Self {
            states,
            draws: Vec::new(),
            root_seed,
            producer_plugin: producer_plugin.to_owned(),
            keyed: keyed.clone(),
        })
    }

    pub(crate) fn range(
        &mut self,
        key: &RandomStreamKey,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<u64, CanwuError> {
        if upper_exclusive == 0 || purpose.trim().is_empty() || purpose != purpose.trim() {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draws require a positive bound and canonical purpose",
            ));
        }
        let Some(state) = self.states.get_mut(key) else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredRandomStream,
                format!(
                    "random stream {}.{}@{} was not declared by this system",
                    key.namespace, key.name, key.version
                ),
            ));
        };
        let next_position = state.position.checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "random stream position is exhausted",
            )
        })?;
        let position = state.position;
        let mut generator = DeterministicRng::from_seed(state.generator_state);
        let value = match state.algorithm {
            RandomAlgorithm::SplitMix64V1 => generator.range_modulo(upper_exclusive),
            RandomAlgorithm::SplitMix64V2 => generator.range(upper_exclusive),
        };
        state.position = next_position;
        state.generator_state = generator.state();
        self.draws.push(PendingRandomDraw {
            stream: key.clone(),
            address: RandomDrawAddress::Sequential { position },
            operation_evidence: None,
            upper_exclusive,
            value,
            purpose: purpose.to_owned(),
        });
        Ok(value)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn range_for_operation(
        &mut self,
        key: &RandomStreamKey,
        evidence: EvidenceRef,
        operation_kind: &str,
        application_operation_id: &str,
        target: RandomOperationTarget,
        draw_slot: u32,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<u64, CanwuError> {
        if !self.states.contains_key(key) {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredRandomStream,
                format!(
                    "random stream {}.{}@{} was not declared by this system",
                    key.namespace, key.name, key.version
                ),
            ));
        }
        let address = RandomOperationAddressV1 {
            producer_plugin: self.producer_plugin.clone(),
            operation_kind: operation_kind.to_owned(),
            application_operation_id: application_operation_id.to_owned(),
            target,
            draw_slot,
        };
        validate_operation_inputs(key, &address, upper_exclusive, purpose)?;
        let index_key = (key.clone(), address.clone());
        let purpose_hash = purpose_hash_hex_v1(purpose)?;
        if let Some(existing) = self.keyed.get(&index_key) {
            if existing.operation_evidence == evidence
                && existing.upper_exclusive == upper_exclusive
                && existing.purpose_hash == purpose_hash
            {
                return Ok(existing.value);
            }
            return Err(CanwuError::new(
                ErrorCode::RandomOperationConflict,
                "operation-keyed random address was reused with different evidence, bound, or purpose",
            ));
        }
        let value = operation_value_v1(self.root_seed, key, &address, upper_exclusive, purpose)?;
        let memo = KeyedDrawMemo {
            operation_evidence: evidence.clone(),
            upper_exclusive,
            value,
            purpose_hash,
        };
        self.keyed.insert(index_key, memo);
        self.draws.push(PendingRandomDraw {
            stream: key.clone(),
            address: RandomDrawAddress::OperationV1(address),
            operation_evidence: Some(evidence),
            upper_exclusive,
            value,
            purpose: purpose.to_owned(),
        });
        Ok(value)
    }

    pub(crate) fn finish(self) -> RandomExecution {
        RandomExecution {
            states: self.states,
            draws: self.draws,
        }
    }
}

pub(crate) fn retained_keyed_draws(
    draws: &[RandomDrawRecord],
) -> Result<KeyedDrawIndex, CanwuError> {
    let mut index = BTreeMap::new();
    for draw in draws {
        let RandomDrawAddress::OperationV1(address) = &draw.address else {
            continue;
        };
        let evidence = draw.operation_evidence.clone().ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "operation-keyed draw is missing its evidence reference",
            )
        })?;
        validate_operation_inputs(&draw.stream, address, draw.upper_exclusive, &draw.purpose)?;
        let key = (draw.stream.clone(), address.clone());
        let memo = KeyedDrawMemo {
            operation_evidence: evidence,
            upper_exclusive: draw.upper_exclusive,
            value: draw.value,
            purpose_hash: purpose_hash_hex_v1(&draw.purpose)?,
        };
        if index.insert(key, memo).is_some() {
            return Err(CanwuError::new(
                ErrorCode::RandomOperationConflict,
                "random journal contains a duplicate operation-keyed address",
            ));
        }
    }
    Ok(index)
}

pub(crate) fn keyed_draws_with_reservations(
    draws: &[RandomDrawRecord],
    reservations: &[KeyedDrawReservation],
) -> Result<KeyedDrawIndex, CanwuError> {
    let mut index = retained_keyed_draws(draws)?;
    for reservation in reservations {
        validate_operation_address(
            &reservation.stream,
            &reservation.address,
            reservation.upper_exclusive,
        )?;
        if reservation.purpose_hash.len() != 64
            || reservation
                .purpose_hash
                .bytes()
                .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
            || reservation.draw_receipt.evidence != EvidenceRef::RandomDraw(reservation.draw_id)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "keyed draw reservation has an invalid purpose hash or draw receipt",
            ));
        }
        let key = (reservation.stream.clone(), reservation.address.clone());
        let memo = KeyedDrawMemo {
            operation_evidence: reservation.operation_evidence.clone(),
            upper_exclusive: reservation.upper_exclusive,
            value: reservation.result,
            purpose_hash: reservation.purpose_hash.clone(),
        };
        if index.insert(key, memo).is_some() {
            return Err(CanwuError::new(
                ErrorCode::RandomOperationConflict,
                "keyed draw reservation overlaps retained or reserved evidence",
            ));
        }
    }
    Ok(index)
}

pub(crate) fn extend_keyed_draws(
    index: &mut KeyedDrawIndex,
    draws: &[PendingRandomDraw],
) -> Result<(), CanwuError> {
    for draw in draws {
        let RandomDrawAddress::OperationV1(address) = &draw.address else {
            continue;
        };
        let evidence = draw.operation_evidence.clone().ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "pending operation-keyed draw is missing evidence",
            )
        })?;
        let memo = KeyedDrawMemo {
            operation_evidence: evidence,
            upper_exclusive: draw.upper_exclusive,
            value: draw.value,
            purpose_hash: purpose_hash_hex_v1(&draw.purpose)?,
        };
        if index
            .insert((draw.stream.clone(), address.clone()), memo)
            .is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::RandomOperationConflict,
                "pending random execution duplicated an operation-keyed address",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_operation_draw(
    root_seed: u64,
    draw: &RandomDrawRecord,
) -> Result<(), CanwuError> {
    match (&draw.address, &draw.operation_evidence) {
        (RandomDrawAddress::Sequential { .. }, None) => Ok(()),
        (RandomDrawAddress::Sequential { .. }, Some(_))
        | (RandomDrawAddress::OperationV1(_), None) => Err(CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "random address and operation evidence are inconsistent",
        )),
        (RandomDrawAddress::OperationV1(address), Some(_)) => {
            let expected = operation_value_v1(
                root_seed,
                &draw.stream,
                address,
                draw.upper_exclusive,
                &draw.purpose,
            )?;
            if expected != draw.value {
                return Err(CanwuError::new(
                    ErrorCode::InvalidRandomDraw,
                    "operation-keyed random value does not match its exact V1 address",
                ));
            }
            Ok(())
        }
    }
}

fn operation_value_v1(
    root_seed: u64,
    key: &RandomStreamKey,
    address: &RandomOperationAddressV1,
    upper_exclusive: u64,
    purpose: &str,
) -> Result<u64, CanwuError> {
    validate_operation_inputs(key, address, upper_exclusive, purpose)?;
    let purpose_hash = purpose_hash_v1(purpose)?;
    for candidate_index in 0..=u32::MAX {
        let bytes = operation_input_v1(
            root_seed,
            key,
            address,
            upper_exclusive,
            &purpose_hash,
            candidate_index,
        )?;
        let digest = blake3::hash(&bytes);
        let mut candidate_bytes = [0_u8; 8];
        candidate_bytes.copy_from_slice(&digest.as_bytes()[..8]);
        let candidate = u64::from_le_bytes(candidate_bytes);
        let range = 1_u128 << 64;
        let bound = u128::from(upper_exclusive);
        let accept_limit = (range / bound) * bound;
        if u128::from(candidate) < accept_limit {
            return Ok(candidate % upper_exclusive);
        }
    }
    Err(CanwuError::new(
        ErrorCode::IdentifierExhausted,
        "operation-keyed random candidate space is exhausted",
    ))
}

fn validate_operation_inputs(
    key: &RandomStreamKey,
    address: &RandomOperationAddressV1,
    upper_exclusive: u64,
    purpose: &str,
) -> Result<(), CanwuError> {
    validate_operation_address(key, address, upper_exclusive)?;
    validate_operation_text(purpose)
}

fn validate_operation_address(
    key: &RandomStreamKey,
    address: &RandomOperationAddressV1,
    upper_exclusive: u64,
) -> Result<(), CanwuError> {
    if upper_exclusive == 0 || key.version == 0 {
        return Err(CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "operation-keyed draws require positive stream version and bound",
        ));
    }
    for value in [
        key.namespace.as_str(),
        key.name.as_str(),
        address.producer_plugin.as_str(),
        address.operation_kind.as_str(),
        address.application_operation_id.as_str(),
    ] {
        validate_operation_text(value)?;
    }
    validate_target(&address.target)
}

fn validate_target(target: &RandomOperationTarget) -> Result<(), CanwuError> {
    match target {
        RandomOperationTarget::Entity(EntityRef::Domain(reference)) => {
            validate_operation_text(&reference.kind.namespace)?;
            validate_operation_text(&reference.kind.name)?;
            validate_operation_text(&reference.id)?;
        }
        RandomOperationTarget::DomainRecord { record, version } => {
            if *version == 0 {
                return Err(CanwuError::new(
                    ErrorCode::InvalidRandomDraw,
                    "operation-keyed domain record target requires a positive version",
                ));
            }
            validate_operation_text(&record.kind.namespace)?;
            validate_operation_text(&record.kind.name)?;
            validate_operation_text(&record.id)?;
        }
        RandomOperationTarget::CanonicalKey(value) => validate_operation_text(value)?,
        RandomOperationTarget::Entity(_) | RandomOperationTarget::KnowledgeHolder(_) => {}
    }
    if let RandomOperationTarget::KnowledgeHolder(KnowledgeHolderRef::Entity(EntityRef::Domain(
        reference,
    ))) = target
    {
        validate_operation_text(&reference.kind.namespace)?;
        validate_operation_text(&reference.kind.name)?;
        validate_operation_text(&reference.id)?;
    }
    Ok(())
}

fn validate_operation_text(value: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value != value.trim()
        || value.len() > OPERATION_TEXT_BYTES
        || u32::try_from(value.len()).is_err()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "operation-keyed random text is empty, non-canonical, or too long",
        ));
    }
    Ok(())
}

fn purpose_hash_v1(purpose: &str) -> Result<[u8; 32], CanwuError> {
    validate_operation_text(purpose)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(PURPOSE_DOMAIN);
    bytes.push(0);
    put_text(&mut bytes, purpose)?;
    Ok(*blake3::hash(&bytes).as_bytes())
}

pub(crate) fn purpose_hash_hex_v1(purpose: &str) -> Result<String, CanwuError> {
    Ok(blake3::Hash::from_bytes(purpose_hash_v1(purpose)?)
        .to_hex()
        .to_string())
}

fn operation_input_v1(
    root_seed: u64,
    key: &RandomStreamKey,
    address: &RandomOperationAddressV1,
    upper_exclusive: u64,
    purpose_hash: &[u8; 32],
    candidate_index: u32,
) -> Result<Vec<u8>, CanwuError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(OPERATION_DOMAIN);
    bytes.push(0);
    bytes.push(1);
    bytes.extend_from_slice(&root_seed.to_le_bytes());
    put_text(&mut bytes, &key.namespace)?;
    put_text(&mut bytes, &key.name)?;
    bytes.extend_from_slice(&key.version.to_le_bytes());
    put_text(&mut bytes, &address.producer_plugin)?;
    put_text(&mut bytes, &address.operation_kind)?;
    put_text(&mut bytes, &address.application_operation_id)?;
    encode_target(&mut bytes, &address.target)?;
    bytes.extend_from_slice(&address.draw_slot.to_le_bytes());
    bytes.extend_from_slice(&upper_exclusive.to_le_bytes());
    bytes.extend_from_slice(purpose_hash);
    bytes.extend_from_slice(&candidate_index.to_le_bytes());
    Ok(bytes)
}

fn put_text(bytes: &mut Vec<u8>, value: &str) -> Result<(), CanwuError> {
    validate_operation_text(value)?;
    let length = u32::try_from(value.len()).map_err(|_| {
        CanwuError::new(
            ErrorCode::InvalidRandomDraw,
            "operation-keyed text length exceeds u32",
        )
    })?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn encode_target(bytes: &mut Vec<u8>, target: &RandomOperationTarget) -> Result<(), CanwuError> {
    match target {
        RandomOperationTarget::Entity(entity) => {
            bytes.push(1);
            encode_entity(bytes, entity)?;
        }
        RandomOperationTarget::DomainRecord { record, version } => {
            if *version == 0 {
                return Err(CanwuError::new(
                    ErrorCode::InvalidRandomDraw,
                    "operation-keyed domain record target requires a positive version",
                ));
            }
            bytes.push(2);
            put_text(bytes, &record.kind.namespace)?;
            put_text(bytes, &record.kind.name)?;
            put_text(bytes, &record.id)?;
            bytes.extend_from_slice(&version.to_le_bytes());
        }
        RandomOperationTarget::KnowledgeHolder(holder) => {
            bytes.push(3);
            match holder {
                KnowledgeHolderRef::Person(person) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&person.get().to_le_bytes());
                }
                KnowledgeHolderRef::Entity(entity) => {
                    bytes.push(2);
                    encode_entity(bytes, entity)?;
                }
            }
        }
        RandomOperationTarget::CanonicalKey(value) => {
            bytes.push(4);
            put_text(bytes, value)?;
        }
    }
    Ok(())
}

fn encode_entity(bytes: &mut Vec<u8>, entity: &EntityRef) -> Result<(), CanwuError> {
    match entity {
        EntityRef::Army(id) => {
            bytes.push(1);
            bytes.extend_from_slice(&id.get().to_le_bytes());
        }
        EntityRef::Government(id) => {
            bytes.push(2);
            bytes.extend_from_slice(&id.get().to_le_bytes());
        }
        EntityRef::Organization(id) => {
            bytes.push(3);
            bytes.extend_from_slice(&id.get().to_le_bytes());
        }
        EntityRef::Person(id) => {
            bytes.push(4);
            bytes.extend_from_slice(&id.get().to_le_bytes());
        }
        EntityRef::Resource(id) => {
            bytes.push(5);
            bytes.extend_from_slice(&id.get().to_le_bytes());
        }
        EntityRef::Route(id) => {
            bytes.push(6);
            bytes.extend_from_slice(&id.get().to_le_bytes());
        }
        EntityRef::Territory(id) => {
            bytes.push(7);
            bytes.extend_from_slice(&id.get().to_le_bytes());
        }
        EntityRef::Domain(reference) => {
            bytes.push(8);
            put_text(bytes, &reference.kind.namespace)?;
            put_text(bytes, &reference.kind.name)?;
            put_text(bytes, &reference.id)?;
        }
    }
    Ok(())
}

pub(crate) fn derive_stream_seed(root_seed: u64, key: &RandomStreamKey) -> u64 {
    if key.namespace == "canwu.core" && key.name == "knowledge-report-delay" && key.version == 1 {
        return root_seed;
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(STREAM_DERIVATION_DOMAIN);
    hasher.update(&root_seed.to_le_bytes());
    update_text(&mut hasher, &key.namespace);
    update_text(&mut hasher, &key.name);
    hasher.update(&key.version.to_le_bytes());
    let mut seed = [0_u8; 8];
    seed.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
    u64::from_le_bytes(seed)
}

pub(crate) fn core_report_delay_stream() -> RandomStreamKey {
    RandomStreamKey::new("canwu.core", "knowledge-report-delay", 1)
}

fn update_text(hasher: &mut blake3::Hasher, value: &str) {
    let length = u64::try_from(value.len()).unwrap_or(u64::MAX);
    hasher.update(&length.to_le_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_core::{GovernmentId, KnowledgeHolderRef, PersonId};

    fn fixture_stream() -> RandomStreamKey {
        RandomStreamKey::new("fixture.random", "resolution", 1)
    }

    fn fixture_address(target: RandomOperationTarget) -> RandomOperationAddressV1 {
        RandomOperationAddressV1 {
            producer_plugin: "fixture-random".to_owned(),
            operation_kind: "resolve".to_owned(),
            application_operation_id: "operation-alpha".to_owned(),
            target,
            draw_slot: 3,
        }
    }

    #[test]
    fn operation_v1_golden_vectors_cover_every_target_encoding() {
        let stream = fixture_stream();
        let targets = [
            RandomOperationTarget::Entity(EntityRef::Government(GovernmentId::new(9))),
            RandomOperationTarget::DomainRecord {
                record: DomainRecordRef {
                    kind: canwu_core::DomainRecordKind::new("fixture", "record"),
                    id: "r-7".to_owned(),
                },
                version: 4,
            },
            RandomOperationTarget::KnowledgeHolder(KnowledgeHolderRef::Person(PersonId::new(5))),
            RandomOperationTarget::CanonicalKey("键-α".to_owned()),
        ];
        let values = targets
            .into_iter()
            .map(|target| {
                let address = fixture_address(target);
                let purpose_hash = purpose_hash_v1("stable outcome").expect("purpose should hash");
                let input = operation_input_v1(
                    0x0102_0304_0506_0708,
                    &stream,
                    &address,
                    10_000,
                    &purpose_hash,
                    0,
                )
                .expect("golden input should encode");
                let digest = blake3::hash(&input).to_hex().to_string();
                let value = operation_value_v1(
                    0x0102_0304_0506_0708,
                    &stream,
                    &address,
                    10_000,
                    "stable outcome",
                )
                .expect("golden vector should encode");
                (input, digest, value)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            values
                .iter()
                .map(|(_, digest, _)| digest.as_str())
                .collect::<Vec<_>>(),
            vec![
                "55b21978711c8d81a42bdacef84c8b22e16bf5e2a36135097f850e658ed86a74",
                "4ac40330c89fc0bce339b16c59fa25166f714b664f4fd3e9e252cb9960b979d7",
                "c23814e1a5343aba6e00b88c1267aef583d6adf2385173b80268f14c6d34f594",
                "8f84ed6f5700db025dff8e59966c8f14d9c2a262904726688ea08d9fd043816d",
            ]
        );
        assert_eq!(
            values
                .iter()
                .map(|(_, _, value)| *value)
                .collect::<Vec<_>>(),
            vec![8389, 6730, 1186, 1231]
        );
        assert_eq!(
            values[0].0.iter().fold(String::new(), |mut output, byte| {
                use std::fmt::Write as _;
                write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
                output
            }),
            "63616e77752e72616e646f6d2e6f7065726174696f6e2e7631000108070605040302010e000000666978747572652e72616e646f6d0a0000007265736f6c7574696f6e010000000e000000666978747572652d72616e646f6d070000007265736f6c76650f0000006f7065726174696f6e2d616c70686101020900000000000000030000001027000000000000784ea13c085b19a2d0420898b8c3848517c24f30b1d82658617e80f99edff43a00000000"
        );
    }

    #[test]
    fn keyed_retry_is_idempotent_conflicts_fail_and_sequential_state_does_not_move() {
        let stream = fixture_stream();
        let state = RandomStreamState::initial(41, stream.clone());
        let mut session = RandomSession::new(
            &BTreeMap::from([(stream.clone(), state)]),
            std::slice::from_ref(&stream),
            41,
            "fixture-random",
            &BTreeMap::new(),
        )
        .expect("session should initialize");
        let evidence = EvidenceRef::Ingress(canwu_core::IngressId::new(2));
        let first = session
            .range_for_operation(
                &stream,
                evidence.clone(),
                "resolve",
                "operation-alpha",
                RandomOperationTarget::CanonicalKey("target".to_owned()),
                0,
                100,
                "resolution",
            )
            .expect("first keyed draw should succeed");
        let retry = session
            .range_for_operation(
                &stream,
                evidence.clone(),
                "resolve",
                "operation-alpha",
                RandomOperationTarget::CanonicalKey("target".to_owned()),
                0,
                100,
                "resolution",
            )
            .expect("exact retry should reuse the result");
        assert_eq!(retry, first);
        let conflict = session
            .range_for_operation(
                &stream,
                evidence,
                "resolve",
                "operation-alpha",
                RandomOperationTarget::CanonicalKey("target".to_owned()),
                0,
                101,
                "resolution",
            )
            .expect_err("changed bound must conflict");
        assert_eq!(conflict.code, ErrorCode::RandomOperationConflict);
        let purpose_conflict = session
            .range_for_operation(
                &stream,
                EvidenceRef::Ingress(canwu_core::IngressId::new(2)),
                "resolve",
                "operation-alpha",
                RandomOperationTarget::CanonicalKey("target".to_owned()),
                0,
                100,
                "different-resolution",
            )
            .expect_err("changed purpose must conflict");
        assert_eq!(purpose_conflict.code, ErrorCode::RandomOperationConflict);
        let execution = session.finish();
        assert_eq!(execution.draws.len(), 1);
        assert_eq!(execution.states[&stream].position, 0);
    }

    #[test]
    fn evidence_renumbering_and_unrelated_operations_do_not_change_keyed_entropy() {
        let stream = fixture_stream();
        let run = |evidence, include_unrelated| {
            let state = RandomStreamState::initial(91, stream.clone());
            let mut session = RandomSession::new(
                &BTreeMap::from([(stream.clone(), state)]),
                std::slice::from_ref(&stream),
                91,
                "fixture-random",
                &BTreeMap::new(),
            )
            .expect("session should initialize");
            if include_unrelated {
                session
                    .range_for_operation(
                        &stream,
                        EvidenceRef::Ingress(canwu_core::IngressId::new(1)),
                        "resolve",
                        "unrelated-operation",
                        RandomOperationTarget::CanonicalKey("unrelated".to_owned()),
                        0,
                        1_000,
                        "unrelated-purpose",
                    )
                    .expect("unrelated keyed operation should succeed");
            }
            session
                .range_for_operation(
                    &stream,
                    EvidenceRef::Ingress(canwu_core::IngressId::new(evidence)),
                    "resolve",
                    "operation-alpha",
                    RandomOperationTarget::CanonicalKey("target".to_owned()),
                    0,
                    1_000,
                    "resolution",
                )
                .expect("target keyed operation should succeed")
        };
        let baseline = run(2, false);
        assert_eq!(run(99, false), baseline);
        assert_eq!(run(2, true), baseline);
    }

    #[test]
    fn producer_namespace_is_encoded_and_rejection_reduction_retries() {
        let stream = fixture_stream();
        let mut first = fixture_address(RandomOperationTarget::CanonicalKey("target".to_owned()));
        let mut second = first.clone();
        second.producer_plugin = "fixture-random-b".to_owned();
        assert_ne!(first, second);
        let purpose_hash = purpose_hash_v1("resolution").expect("purpose should hash");
        assert_ne!(
            operation_input_v1(17, &stream, &first, 100, &purpose_hash, 0)
                .expect("first producer input"),
            operation_input_v1(17, &stream, &second, 100, &purpose_hash, 0)
                .expect("second producer input")
        );

        first.application_operation_id = "rejection-reduction".to_owned();
        let upper_exclusive = (1_u64 << 63) + 1;
        let range = 1_u128 << 64;
        let bound = u128::from(upper_exclusive);
        let accept_limit = (range / bound) * bound;
        let purpose = (0_u32..10_000)
            .map(|index| format!("retry-purpose-{index}"))
            .find(|purpose| {
                let purpose_hash = purpose_hash_v1(purpose).expect("purpose should hash");
                let bytes =
                    operation_input_v1(17, &stream, &first, upper_exclusive, &purpose_hash, 0)
                        .expect("candidate zero should encode");
                let digest = blake3::hash(&bytes);
                let mut candidate_bytes = [0_u8; 8];
                candidate_bytes.copy_from_slice(&digest.as_bytes()[..8]);
                u128::from(u64::from_le_bytes(candidate_bytes)) >= accept_limit
            })
            .expect("fixture search should find a rejected candidate zero");
        let value = operation_value_v1(17, &stream, &first, upper_exclusive, &purpose)
            .expect("rejection reduction must find a later candidate");
        assert!(value < upper_exclusive);
    }

    #[test]
    fn sequential_rejection_sampling_replays_the_actual_generator_state() {
        let stream = fixture_stream();
        let upper_exclusive = (1_u64 << 63) + 1;
        let rejection_threshold = upper_exclusive.wrapping_neg() % upper_exclusive;
        let (root_seed, initial) = (1_u64..)
            .find_map(|root_seed| {
                let initial = RandomStreamState::initial(root_seed, stream.clone());
                let mut probe = DeterministicRng::from_seed(initial.generator_state);
                (probe.next_u64() < rejection_threshold).then_some((root_seed, initial))
            })
            .expect("fixture search should find a sequential rejection");
        let mut session = RandomSession::new(
            &BTreeMap::from([(stream.clone(), initial.clone())]),
            std::slice::from_ref(&stream),
            root_seed,
            "fixture-random",
            &BTreeMap::new(),
        )
        .expect("session should initialize");
        let value = session
            .range(&stream, upper_exclusive, "sequential rejection")
            .expect("sequential rejection draw should succeed");
        let execution = session.finish();
        let final_state = &execution.states[&stream];

        let mut replay = DeterministicRng::from_seed(initial.generator_state);
        assert_eq!(replay.range(upper_exclusive), value);
        assert_eq!(replay.state(), final_state.generator_state);
        assert_eq!(final_state.position, 1);
        assert_ne!(
            final_state.generator_state,
            DeterministicRng::state_after(initial.seed, final_state.position)
        );
    }
}
