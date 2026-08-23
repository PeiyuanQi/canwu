use super::EntityRef;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ActorNotFound,
    ArchiveNotReady,
    ArmyNotFound,
    DestinationNotFound,
    DuplicateBoundaryWriter,
    DuplicateDomainRecord,
    DuplicateDomainRecordKind,
    DuplicatePlugin,
    DuplicatePluginCommand,
    DuplicatePluginIngress,
    DuplicatePluginSystem,
    DuplicateStateOwner,
    DuplicateReservationOfferer,
    EntityNotFound,
    DomainRecordNotFound,
    DomainRecordReferenced,
    DomainRecordVersionConflict,
    InvalidAuthority,
    InvalidArchive,
    InvalidBoundary,
    InvalidDecision,
    InvalidDuration,
    InvalidDomainRecord,
    InvalidKnowledgeHolder,
    InvalidKnowledgeRecord,
    InvalidKnowledgeSchema,
    InvalidKnowledgeAuthority,
    KnowledgeLimitExceeded,
    KnowledgeReadCutUnavailable,
    KnowledgeRecordNotFound,
    UndeclaredKnowledgeWrite,
    EvidenceUnavailable,
    EvidenceContentUnavailable,
    DuplicateKnowledgeRecordKind,
    InvalidRandomOperationEvidence,
    RandomOperationConflict,
    IdempotencyConflict,
    InteractionReadOnly,
    InvalidPayload,
    InvalidPluginRegistration,
    InvalidRandomDraw,
    InvalidRandomStream,
    InvalidRunConfiguration,
    InvalidRunManifest,
    InvalidSnapshot,
    IdentifierExhausted,
    LegacyReplayUnavailable,
    LateIngress,
    MissingIdempotencyKey,
    MixedCommandIngress,
    NoRoute,
    PluginCommandNotFound,
    PluginManifestMismatch,
    PluginNotActive,
    PluginPanicked,
    PluginRegistrationClosed,
    ReplayMismatch,
    ReplayEnvironmentMismatch,
    SimulationRevisionConflict,
    SimulationTimeConflict,
    StaleSealToken,
    SynchronousReactionLimit,
    UndeclaredRandomStream,
    UndeclaredStateRead,
    UndeclaredStateWrite,
    UnsupportedSnapshotVersion,
    UnsupportedRandomDrawAddress,
    ValueOutOfRange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CanwuError {
    pub code: ErrorCode,
    pub message: String,
    pub related_entities: Vec<EntityRef>,
}

impl CanwuError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            related_entities: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_entity(mut self, entity: EntityRef) -> Self {
        self.related_entities.push(entity);
        self
    }
}

impl Display for CanwuError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {}",
            error_code_name(&self.code),
            self.message
        )
    }
}

impl Error for CanwuError {}

const fn error_code_name(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::ActorNotFound => "actor_not_found",
        ErrorCode::ArchiveNotReady => "archive_not_ready",
        ErrorCode::ArmyNotFound => "army_not_found",
        ErrorCode::DestinationNotFound => "destination_not_found",
        ErrorCode::DuplicateBoundaryWriter => "duplicate_boundary_writer",
        ErrorCode::DuplicateDomainRecord => "duplicate_domain_record",
        ErrorCode::DuplicateDomainRecordKind => "duplicate_domain_record_kind",
        ErrorCode::DuplicatePlugin => "duplicate_plugin",
        ErrorCode::DuplicatePluginCommand => "duplicate_plugin_command",
        ErrorCode::DuplicatePluginIngress => "duplicate_plugin_ingress",
        ErrorCode::DuplicatePluginSystem => "duplicate_plugin_system",
        ErrorCode::DuplicateStateOwner => "duplicate_state_owner",
        ErrorCode::DuplicateReservationOfferer => "duplicate_reservation_offerer",
        ErrorCode::EntityNotFound => "entity_not_found",
        ErrorCode::DomainRecordNotFound => "domain_record_not_found",
        ErrorCode::DomainRecordReferenced => "domain_record_referenced",
        ErrorCode::DomainRecordVersionConflict => "domain_record_version_conflict",
        ErrorCode::InvalidAuthority => "invalid_authority",
        ErrorCode::InvalidArchive => "invalid_archive",
        ErrorCode::InvalidBoundary => "invalid_boundary",
        ErrorCode::InvalidDecision => "invalid_decision",
        ErrorCode::InvalidDuration => "invalid_duration",
        ErrorCode::InvalidDomainRecord => "invalid_domain_record",
        ErrorCode::InvalidKnowledgeHolder => "invalid_knowledge_holder",
        ErrorCode::InvalidKnowledgeRecord => "invalid_knowledge_record",
        ErrorCode::InvalidKnowledgeSchema => "invalid_knowledge_schema",
        ErrorCode::InvalidKnowledgeAuthority => "invalid_knowledge_authority",
        ErrorCode::KnowledgeLimitExceeded => "knowledge_limit_exceeded",
        ErrorCode::KnowledgeReadCutUnavailable => "knowledge_read_cut_unavailable",
        ErrorCode::KnowledgeRecordNotFound => "knowledge_record_not_found",
        ErrorCode::UndeclaredKnowledgeWrite => "undeclared_knowledge_write",
        ErrorCode::EvidenceUnavailable => "evidence_unavailable",
        ErrorCode::EvidenceContentUnavailable => "evidence_content_unavailable",
        ErrorCode::DuplicateKnowledgeRecordKind => "duplicate_knowledge_record_kind",
        ErrorCode::InvalidRandomOperationEvidence => "invalid_random_operation_evidence",
        ErrorCode::RandomOperationConflict => "random_operation_conflict",
        ErrorCode::IdempotencyConflict => "idempotency_conflict",
        ErrorCode::InteractionReadOnly => "interaction_read_only",
        ErrorCode::InvalidPayload => "invalid_payload",
        ErrorCode::InvalidPluginRegistration => "invalid_plugin_registration",
        ErrorCode::InvalidRandomDraw => "invalid_random_draw",
        ErrorCode::InvalidRandomStream => "invalid_random_stream",
        ErrorCode::InvalidRunConfiguration => "invalid_run_configuration",
        ErrorCode::InvalidRunManifest => "invalid_run_manifest",
        ErrorCode::InvalidSnapshot => "invalid_snapshot",
        ErrorCode::IdentifierExhausted => "identifier_exhausted",
        ErrorCode::LegacyReplayUnavailable => "legacy_replay_unavailable",
        ErrorCode::LateIngress => "late_ingress",
        ErrorCode::MissingIdempotencyKey => "missing_idempotency_key",
        ErrorCode::MixedCommandIngress => "mixed_command_ingress",
        ErrorCode::NoRoute => "no_route",
        ErrorCode::PluginCommandNotFound => "plugin_command_not_found",
        ErrorCode::PluginManifestMismatch => "plugin_manifest_mismatch",
        ErrorCode::PluginNotActive => "plugin_not_active",
        ErrorCode::PluginPanicked => "plugin_panicked",
        ErrorCode::PluginRegistrationClosed => "plugin_registration_closed",
        ErrorCode::ReplayMismatch => "replay_mismatch",
        ErrorCode::ReplayEnvironmentMismatch => "replay_environment_mismatch",
        ErrorCode::SimulationRevisionConflict => "simulation_revision_conflict",
        ErrorCode::SimulationTimeConflict => "simulation_time_conflict",
        ErrorCode::StaleSealToken => "stale_seal_token",
        ErrorCode::SynchronousReactionLimit => "synchronous_reaction_limit",
        ErrorCode::UndeclaredRandomStream => "undeclared_random_stream",
        ErrorCode::UndeclaredStateRead => "undeclared_state_read",
        ErrorCode::UndeclaredStateWrite => "undeclared_state_write",
        ErrorCode::UnsupportedSnapshotVersion => "unsupported_snapshot_version",
        ErrorCode::UnsupportedRandomDrawAddress => "unsupported_random_draw_address",
        ErrorCode::ValueOutOfRange => "value_out_of_range",
    }
}
