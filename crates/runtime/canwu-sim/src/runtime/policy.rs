use super::{CanwuError, ErrorCode, canonical_hash};
use canwu_core::{EntityRef, PersonId};
use serde::{Deserialize, Serialize};

pub const RUN_CONFIGURATION_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPurpose {
    Play,
    HistoricalSimulation,
    Validation,
    Replay,
    DeveloperDiagnostic,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerPolicy {
    HumanRoleBound,
    NoHuman,
    ReplayController,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatPolicy {
    CharacterBound,
    InstitutionBound,
    ObserverSeat,
    AdvisorSeat,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationPolicy {
    ActorBound,
    PublicObserver,
    ResearchFull,
    DeveloperDiagnostic,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionPolicy {
    EraInternalCommands,
    ReadOnly,
    VersionedExperiment,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TracePolicy {
    Minimal,
    Causal,
    Formula,
    FullResearch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SeatBinding {
    pub seat_id: String,
    pub controller_id: String,
    pub actor: Option<PersonId>,
    pub institution: Option<EntityRef>,
    pub permission_profile_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunConfiguration {
    pub format_version: u32,
    pub purpose: RunPurpose,
    pub controller: ControllerPolicy,
    pub seat: SeatPolicy,
    pub observation: ObservationPolicy,
    pub interaction: InteractionPolicy,
    pub trace: TracePolicy,
    pub seat_binding: Option<SeatBinding>,
    #[serde(default)]
    pub declared_interventions: Vec<String>,
    pub diagnostic_commands_enabled: bool,
    pub require_idempotency_keys: bool,
}

/// Command-relevant policy deliberately omits run purpose, observation, and
/// trace so authoritative handlers cannot branch on presentation-only inputs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "provenance", rename_all = "snake_case")]
pub enum CommandPolicyContext {
    Declared {
        format_version: u32,
        controller: ControllerPolicy,
        seat: SeatPolicy,
        interaction: InteractionPolicy,
        diagnostic_commands_enabled: bool,
    },
    CompatibilityV1,
    ManifestOnlyV1,
    LegacyUnspecified,
}

impl RunConfiguration {
    #[must_use]
    pub fn play_as_character(
        seat_id: impl Into<String>,
        controller_id: impl Into<String>,
        actor: PersonId,
        permission_profile_id: impl Into<String>,
    ) -> Self {
        Self {
            format_version: RUN_CONFIGURATION_FORMAT_VERSION,
            purpose: RunPurpose::Play,
            controller: ControllerPolicy::HumanRoleBound,
            seat: SeatPolicy::CharacterBound,
            observation: ObservationPolicy::ActorBound,
            interaction: InteractionPolicy::EraInternalCommands,
            trace: TracePolicy::Causal,
            seat_binding: Some(SeatBinding {
                seat_id: seat_id.into(),
                controller_id: controller_id.into(),
                actor: Some(actor),
                institution: None,
                permission_profile_id: permission_profile_id.into(),
            }),
            declared_interventions: Vec::new(),
            diagnostic_commands_enabled: false,
            require_idempotency_keys: true,
        }
    }

    #[must_use]
    pub fn read_only_observer() -> Self {
        Self {
            format_version: RUN_CONFIGURATION_FORMAT_VERSION,
            purpose: RunPurpose::HistoricalSimulation,
            controller: ControllerPolicy::NoHuman,
            seat: SeatPolicy::None,
            observation: ObservationPolicy::PublicObserver,
            interaction: InteractionPolicy::ReadOnly,
            trace: TracePolicy::Causal,
            seat_binding: None,
            declared_interventions: Vec::new(),
            diagnostic_commands_enabled: false,
            require_idempotency_keys: true,
        }
    }

    #[must_use]
    pub fn replay_as_character(
        seat_id: impl Into<String>,
        recorded_controller_id: impl Into<String>,
        actor: PersonId,
        permission_profile_id: impl Into<String>,
    ) -> Self {
        Self {
            format_version: RUN_CONFIGURATION_FORMAT_VERSION,
            purpose: RunPurpose::Replay,
            controller: ControllerPolicy::ReplayController,
            seat: SeatPolicy::CharacterBound,
            observation: ObservationPolicy::ActorBound,
            interaction: InteractionPolicy::ReadOnly,
            trace: TracePolicy::Causal,
            seat_binding: Some(SeatBinding {
                seat_id: seat_id.into(),
                controller_id: recorded_controller_id.into(),
                actor: Some(actor),
                institution: None,
                permission_profile_id: permission_profile_id.into(),
            }),
            declared_interventions: Vec::new(),
            diagnostic_commands_enabled: false,
            require_idempotency_keys: true,
        }
    }

    pub(crate) fn canonicalize(&mut self) {
        self.declared_interventions.sort();
        self.declared_interventions.dedup();
    }

    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != RUN_CONFIGURATION_FORMAT_VERSION {
            return invalid_configuration(format!(
                "run configuration format {} is unsupported",
                self.format_version
            ));
        }
        if self
            .declared_interventions
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
            || self
                .declared_interventions
                .iter()
                .any(|value| !canonical_text(value))
        {
            return invalid_configuration(
                "declared interventions must be unique, canonical, and sorted",
            );
        }

        match self.seat {
            SeatPolicy::CharacterBound => {
                let Some(binding) = &self.seat_binding else {
                    return invalid_configuration(
                        "a character-bound seat requires an exact seat binding",
                    );
                };
                if binding.actor.is_none() || binding.institution.is_some() {
                    return invalid_configuration(
                        "a character-bound seat requires one actor and no institution",
                    );
                }
            }
            SeatPolicy::InstitutionBound => {
                if self
                    .seat_binding
                    .as_ref()
                    .and_then(|binding| binding.institution.as_ref())
                    .is_none()
                {
                    return invalid_configuration(
                        "an institution-bound seat requires an institution binding",
                    );
                }
            }
            SeatPolicy::ObserverSeat | SeatPolicy::AdvisorSeat => {
                if self.controller == ControllerPolicy::HumanRoleBound
                    && self.seat_binding.is_none()
                {
                    return invalid_configuration(
                        "a human observer or advisor seat requires an exact seat binding",
                    );
                }
            }
            SeatPolicy::None if self.seat_binding.is_some() => {
                return invalid_configuration("seat policy none cannot retain a seat binding");
            }
            SeatPolicy::None => {}
        }

        if let Some(binding) = &self.seat_binding
            && (!canonical_text(&binding.seat_id)
                || !canonical_text(&binding.controller_id)
                || !canonical_text(&binding.permission_profile_id))
        {
            return invalid_configuration(
                "seat bindings require canonical seat, controller, and permission-profile IDs",
            );
        }
        if self.controller == ControllerPolicy::HumanRoleBound && self.seat_binding.is_none() {
            return invalid_configuration("human-role-bound runs require a seat binding");
        }
        if self.controller == ControllerPolicy::NoHuman && self.seat_binding.is_some() {
            return invalid_configuration("no-human runs cannot retain a controller seat binding");
        }
        if self.observation == ObservationPolicy::ActorBound && self.seat_binding.is_none() {
            return invalid_configuration("actor-bound observation requires a seat binding");
        }
        if self.observation == ObservationPolicy::ActorBound
            && self.interaction == InteractionPolicy::EraInternalCommands
            && (self.controller != ControllerPolicy::HumanRoleBound
                || !matches!(
                    self.seat,
                    SeatPolicy::CharacterBound | SeatPolicy::InstitutionBound
                ))
        {
            return invalid_configuration(
                "actor-bound command runs require a human character or institution seat",
            );
        }
        if self.observation == ObservationPolicy::PublicObserver
            && self.interaction != InteractionPolicy::ReadOnly
        {
            return invalid_configuration("public-observer runs must be read-only");
        }
        if self.observation == ObservationPolicy::ResearchFull
            && !matches!(
                self.interaction,
                InteractionPolicy::ReadOnly | InteractionPolicy::VersionedExperiment
            )
        {
            return invalid_configuration(
                "research-full runs must be read-only or a versioned experiment",
            );
        }
        if (self.purpose == RunPurpose::Replay
            || self.controller == ControllerPolicy::ReplayController)
            && (self.purpose != RunPurpose::Replay
                || self.controller != ControllerPolicy::ReplayController
                || self.interaction != InteractionPolicy::ReadOnly)
        {
            return invalid_configuration(
                "replay purpose, replay controller, and read-only interaction must be selected together",
            );
        }
        if self.interaction == InteractionPolicy::VersionedExperiment
            && (self.declared_interventions.is_empty()
                || self.observation != ObservationPolicy::ResearchFull
                || !matches!(
                    self.purpose,
                    RunPurpose::HistoricalSimulation
                        | RunPurpose::Validation
                        | RunPurpose::DeveloperDiagnostic
                ))
        {
            return invalid_configuration(
                "versioned experiments require research observation, a research-capable purpose, and declared interventions",
            );
        }
        if (self.purpose == RunPurpose::DeveloperDiagnostic
            || self.observation == ObservationPolicy::DeveloperDiagnostic
            || self.diagnostic_commands_enabled)
            && (self.purpose != RunPurpose::DeveloperDiagnostic
                || self.observation != ObservationPolicy::DeveloperDiagnostic)
        {
            return invalid_configuration(
                "developer diagnostics require both diagnostic purpose and observation policy",
            );
        }
        Ok(())
    }

    pub(crate) fn semantic_hash(&self) -> Result<String, CanwuError> {
        canonical_hash("canwu.run-configuration.v1", self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "provenance",
    content = "configuration",
    rename_all = "snake_case"
)]
pub enum RunConfigurationSnapshot {
    Declared(RunConfiguration),
    CompatibilityV1,
    ManifestOnlyV1,
    LegacyUnspecified,
}

impl RunConfigurationSnapshot {
    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        match self {
            Self::Declared(configuration) => configuration.validate(),
            Self::CompatibilityV1 | Self::ManifestOnlyV1 | Self::LegacyUnspecified => Ok(()),
        }
    }

    pub(crate) fn semantic_hash(&self) -> Result<Option<String>, CanwuError> {
        match self {
            Self::Declared(configuration) => configuration.semantic_hash().map(Some),
            Self::CompatibilityV1 => compatibility_configuration_hash().map(Some),
            Self::ManifestOnlyV1 | Self::LegacyUnspecified => Ok(None),
        }
    }

    #[must_use]
    pub const fn declared(&self) -> Option<&RunConfiguration> {
        match self {
            Self::Declared(configuration) => Some(configuration),
            Self::CompatibilityV1 | Self::ManifestOnlyV1 | Self::LegacyUnspecified => None,
        }
    }

    pub(crate) const fn command_policy(&self) -> CommandPolicyContext {
        match self {
            Self::Declared(configuration) => CommandPolicyContext::Declared {
                format_version: configuration.format_version,
                controller: configuration.controller,
                seat: configuration.seat,
                interaction: configuration.interaction,
                diagnostic_commands_enabled: configuration.diagnostic_commands_enabled,
            },
            Self::CompatibilityV1 => CommandPolicyContext::CompatibilityV1,
            Self::ManifestOnlyV1 => CommandPolicyContext::ManifestOnlyV1,
            Self::LegacyUnspecified => CommandPolicyContext::LegacyUnspecified,
        }
    }
}

pub(crate) fn compatibility_configuration_hash() -> Result<String, CanwuError> {
    #[derive(Serialize)]
    struct CompatibilityConfiguration<'a> {
        scheduler: &'a str,
        settlement: &'a str,
        observation: &'a str,
        trace: &'a str,
    }

    canonical_hash(
        "canwu.default-run-configuration.v1",
        &CompatibilityConfiguration {
            scheduler: "canonical-single-host-v1",
            settlement: "explicit-fourteen-phase-v1",
            observation: "actor-scoped-v1",
            trace: "authoritative-evidence-v1",
        },
    )
}

pub(crate) fn authoritative_configuration_hash() -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.authoritative-run-configuration.v1",
        &"run-purpose and admission/presentation policy are excluded from simulated-state identity; admitted inputs remain authoritative",
    )
}

fn canonical_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}

fn invalid_configuration<T>(message: impl Into<String>) -> Result<T, CanwuError> {
    Err(CanwuError::new(ErrorCode::InvalidRunConfiguration, message))
}
