use canwu_core::{
    CommandRequestId, DecisionRequestId, DecisionTicketId, DecisionTraceId, EntityRef, PersonId,
};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionErrorCode {
    ClosedTicket,
    DuplicateController,
    DuplicateResponse,
    DuplicateTicket,
    DecisionHistoryUnavailable,
    InvalidController,
    InvalidDecision,
    InvalidOption,
    PolicyMismatch,
    TicketNotFound,
    QueryBudgetExceeded,
    VersionConflict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAttemptErrorCode {
    SimulationRevisionConflict,
    CommandRequestConflict,
    EntityUnavailable,
    ClosedTicket,
    DuplicateController,
    DuplicateResponse,
    DuplicateTicket,
    DecisionHistoryUnavailable,
    InvalidController,
    InvalidDecision,
    InvalidOption,
    PolicyMismatch,
    TicketNotFound,
    QueryBudgetExceeded,
    VersionConflict,
}

impl From<DecisionErrorCode> for DecisionAttemptErrorCode {
    fn from(value: DecisionErrorCode) -> Self {
        match value {
            DecisionErrorCode::ClosedTicket => Self::ClosedTicket,
            DecisionErrorCode::DuplicateController => Self::DuplicateController,
            DecisionErrorCode::DuplicateResponse => Self::DuplicateResponse,
            DecisionErrorCode::DuplicateTicket => Self::DuplicateTicket,
            DecisionErrorCode::DecisionHistoryUnavailable => Self::DecisionHistoryUnavailable,
            DecisionErrorCode::InvalidController => Self::InvalidController,
            DecisionErrorCode::InvalidDecision => Self::InvalidDecision,
            DecisionErrorCode::InvalidOption => Self::InvalidOption,
            DecisionErrorCode::PolicyMismatch => Self::PolicyMismatch,
            DecisionErrorCode::TicketNotFound => Self::TicketNotFound,
            DecisionErrorCode::QueryBudgetExceeded => Self::QueryBudgetExceeded,
            DecisionErrorCode::VersionConflict => Self::VersionConflict,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionError {
    pub code: DecisionErrorCode,
    pub message: String,
}

impl DecisionError {
    #[must_use]
    pub fn new(code: DecisionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl Display for DecisionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl Error for DecisionError {}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPolicyKind {
    Utility,
    Rule,
    Human,
    External,
    Llm,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DecisionPolicyIdentity {
    pub kind: DecisionPolicyKind,
    pub id: String,
    pub version: String,
}

impl DecisionPolicyIdentity {
    #[must_use]
    pub fn new(
        kind: DecisionPolicyKind,
        id: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            id: id.into(),
            version: version.into(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.id, "policy ID")?;
        require_text(&self.version, "policy version")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAuthority {
    Actor {
        actor: PersonId,
    },
    Institution {
        institution: EntityRef,
        responsible_actor: Option<PersonId>,
    },
    Council {
        council_id: String,
    },
    NoResponsibleActor {
        reason: String,
    },
}

impl DecisionAuthority {
    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        match self {
            Self::Actor { .. } | Self::Institution { .. } => Ok(()),
            Self::Council { council_id } => require_identifier(council_id, "council ID"),
            Self::NoResponsibleActor { reason } => require_text(reason, "authority reason"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionControllerBinding {
    pub id: String,
    pub policy: DecisionPolicyIdentity,
    pub authority: DecisionAuthority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_subject: Option<EntityRef>,
}

impl DecisionControllerBinding {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        policy: DecisionPolicyIdentity,
        authority: DecisionAuthority,
    ) -> Self {
        Self {
            id: id.into(),
            policy,
            authority,
            seat_id: None,
            permission_profile_id: None,
            command_subject: None,
        }
    }

    #[must_use]
    pub fn with_seat(
        mut self,
        seat_id: impl Into<String>,
        permission_profile_id: impl Into<String>,
    ) -> Self {
        self.seat_id = Some(seat_id.into());
        self.permission_profile_id = Some(permission_profile_id.into());
        self
    }

    #[must_use]
    pub fn with_command_subject(mut self, command_subject: EntityRef) -> Self {
        self.command_subject = Some(command_subject);
        self
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.id, "controller ID")?;
        self.policy.validate()?;
        self.authority.validate()?;
        if self.seat_id.is_some() != self.permission_profile_id.is_some() {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidController,
                "seat ID and permission-profile ID must be supplied together",
            ));
        }
        if let Some(seat_id) = &self.seat_id {
            require_identifier(seat_id, "seat ID")?;
        }
        if let Some(profile) = &self.permission_profile_id {
            require_identifier(profile, "permission-profile ID")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionContext {
    pub schema: String,
    pub payload: Value,
}

impl DecisionContext {
    #[must_use]
    pub fn new(schema: impl Into<String>, payload: Value) -> Self {
        Self {
            schema: schema.into(),
            payload,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.schema, "decision context schema")
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAction {
    #[default]
    None,
    /// A serialized `canwu_sim::Command`. The controller supplies issuer and
    /// authority; a policy can only select this existing envelope.
    Command { command: Value },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub action: DecisionAction,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub utility_inputs: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

impl DecisionOption {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: String::new(),
            action: DecisionAction::None,
            utility_inputs: BTreeMap::new(),
            blockers: Vec::new(),
            metadata: Value::Null,
        }
    }

    #[must_use]
    pub fn with_command(mut self, command: Value) -> Self {
        self.action = DecisionAction::Command { command };
        self
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.blockers.is_empty()
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.id, "option ID")?;
        require_text(&self.label, "option label")?;
        for key in self.utility_inputs.keys() {
            require_identifier(key, "utility factor")?;
        }
        if self.blockers.iter().any(|value| !is_canonical_text(value)) {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidOption,
                "option blockers must be non-empty canonical text",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionTicketDraft {
    pub id: DecisionTicketId,
    pub definition: String,
    pub decision_maker: EntityRef,
    pub assigned_controller: String,
    pub summary: String,
    pub context: DecisionContext,
    pub options: Vec<DecisionOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<SimTime>,
}

impl DecisionTicketDraft {
    pub(crate) fn validate(&mut self) -> Result<(), DecisionError> {
        if self.id.get() == 0 {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision ticket IDs must be nonzero",
            ));
        }
        require_identifier(&self.definition, "decision definition")?;
        require_identifier(&self.assigned_controller, "assigned controller")?;
        require_text(&self.summary, "decision summary")?;
        self.context.validate()?;
        canonicalize_options(&mut self.options)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DecisionTicketState {
    Open,
    Resolved {
        option_id: String,
        trace_id: DecisionTraceId,
    },
    Cancelled {
        reason: String,
    },
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionTicket {
    pub id: DecisionTicketId,
    pub definition: String,
    pub decision_maker: EntityRef,
    pub assigned_controller: String,
    pub summary: String,
    pub context: DecisionContext,
    pub options: Vec<DecisionOption>,
    pub opened_at: SimTime,
    pub updated_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<SimTime>,
    pub version: u64,
    pub state: DecisionTicketState,
}

impl DecisionTicket {
    #[must_use]
    pub fn option(&self, id: &str) -> Option<&DecisionOption> {
        self.options
            .binary_search_by(|option| option.id.as_str().cmp(id))
            .ok()
            .and_then(|index| self.options.get(index))
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.state, DecisionTicketState::Open)
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.definition, "decision definition")?;
        require_identifier(&self.assigned_controller, "assigned controller")?;
        require_text(&self.summary, "decision summary")?;
        self.context.validate()?;
        let mut options = self.options.clone();
        canonicalize_options(&mut options)?;
        if options != self.options || self.version == 0 || self.updated_at < self.opened_at {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision ticket ordering, version, or timestamps are invalid",
            ));
        }
        if self
            .deadline
            .is_some_and(|deadline| deadline < self.opened_at)
        {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision deadline precedes the ticket opening time",
            ));
        }
        match &self.state {
            DecisionTicketState::Resolved { option_id, .. } if self.option(option_id).is_none() => {
                Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "resolved decision references an unknown option",
                ))
            }
            DecisionTicketState::Cancelled { reason } => {
                require_text(reason, "decision cancellation reason")
            }
            DecisionTicketState::Open
            | DecisionTicketState::Resolved { .. }
            | DecisionTicketState::Expired => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionFactorContribution {
    pub factor: String,
    pub value: i64,
    pub weight: i64,
    pub contribution: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionOptionEvaluation {
    pub option_id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factors: Vec<DecisionFactorContribution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionExternalEvidence {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionOutcome {
    Selected { option_id: String },
    Deferred { reason: String },
    Pending { reason: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyDecision {
    pub outcome: DecisionOutcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluations: Vec<DecisionOptionEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<DecisionExternalEvidence>,
}

impl PolicyDecision {
    #[must_use]
    pub fn selected(option_id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            outcome: DecisionOutcome::Selected {
                option_id: option_id.into(),
            },
            summary: summary.into(),
            evaluations: Vec::new(),
            external: None,
        }
    }

    #[must_use]
    pub fn pending(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            outcome: DecisionOutcome::Pending {
                reason: reason.clone(),
            },
            summary: reason,
            evaluations: Vec::new(),
            external: None,
        }
    }

    pub(crate) fn validate(&self, ticket: &DecisionTicket) -> Result<(), DecisionError> {
        require_text(&self.summary, "policy decision summary")?;
        match &self.outcome {
            DecisionOutcome::Selected { option_id } => {
                let option = ticket.option(option_id).ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidOption,
                        format!("policy selected unknown option {option_id}"),
                    )
                })?;
                if !option.is_available() {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidOption,
                        format!("policy selected blocked option {option_id}"),
                    ));
                }
            }
            DecisionOutcome::Deferred { reason } | DecisionOutcome::Pending { reason } => {
                require_text(reason, "decision outcome reason")?;
            }
        }
        for evaluation in &self.evaluations {
            if ticket.option(&evaluation.option_id).is_none() {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "policy evaluation references an unknown option",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionTrace {
    pub id: DecisionTraceId,
    pub ticket_id: DecisionTicketId,
    pub ticket_version: u64,
    pub controller_id: String,
    pub policy: DecisionPolicyIdentity,
    pub decided_at: SimTime,
    pub outcome: DecisionOutcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluations: Vec<DecisionOptionEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<DecisionExternalEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_request_id: Option<CommandRequestId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DecisionAttemptOutcome {
    Accepted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<DecisionTraceId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_request_id: Option<CommandRequestId>,
    },
    Rejected {
        code: DecisionAttemptErrorCode,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionAttemptRecord {
    pub request_id: DecisionRequestId,
    /// Canonical commitment to the complete admitted decision ingress request.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_commitment: String,
    pub at: SimTime,
    /// Authoritative revision immediately before decision admission.
    pub revision_before: u64,
    pub expected_revision: u64,
    pub outcome: DecisionAttemptOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionMutation {
    RegisterController {
        controller: DecisionControllerBinding,
    },
    Open {
        ticket: DecisionTicketDraft,
    },
    ReplaceOptions {
        ticket_id: DecisionTicketId,
        expected_version: u64,
        context: DecisionContext,
        options: Vec<DecisionOption>,
    },
    Resolve {
        ticket_id: DecisionTicketId,
        expected_version: u64,
        controller_id: String,
        policy: DecisionPolicyIdentity,
        decision: PolicyDecision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_request_id: Option<CommandRequestId>,
    },
    Cancel {
        ticket_id: DecisionTicketId,
        expected_version: u64,
        reason: String,
    },
}

pub(crate) fn canonicalize_options(options: &mut Vec<DecisionOption>) -> Result<(), DecisionError> {
    for option in &mut *options {
        option.blockers.sort();
        option.blockers.dedup();
        option.validate()?;
    }
    options.sort_by(|left, right| left.id.cmp(&right.id));
    if options.is_empty() || options.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidOption,
            "decision options must contain at least one unique option",
        ));
    }
    Ok(())
}

pub(crate) fn require_identifier(value: &str, label: &str) -> Result<(), DecisionError> {
    if !is_canonical_text(value) || value.chars().any(char::is_whitespace) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            format!("{label} must be non-empty canonical text without whitespace"),
        ));
    }
    Ok(())
}

pub(crate) fn require_text(value: &str, label: &str) -> Result<(), DecisionError> {
    if !is_canonical_text(value) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            format!("{label} must be non-empty canonical text"),
        ));
    }
    Ok(())
}

fn is_canonical_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}
