use crate::schema::{
    Audience, Content, DeliveryAttempt, Dispatch, Instance, Release, Representation,
};
use canwu_api::{
    BoundaryId, CommandId, DomainRecordKind, DomainRecordVersionRef, EntityRef, EvidenceRef,
    KnowledgeHolderRef, SimTime, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Versioned admission and fan-out limits committed by the plugin descriptor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InformationLimitsV1 {
    pub max_parent_edges: u32,
    pub max_addressed_recipients: u32,
    pub max_explicit_audience_members: u32,
    pub max_resolved_group_references: u32,
    pub max_delivery_attempts_per_recipient: u32,
    pub max_operation_output_slots: u32,
    pub max_inline_body_bytes: u64,
    pub max_external_resource_bytes: u64,
    pub max_membership_proof_siblings: u32,
    pub max_membership_proof_bytes: u32,
}

impl InformationLimitsV1 {
    pub const FORMAT_VERSION: u32 = 1;
    pub const ONE_TEBIBYTE: u64 = 1_099_511_627_776;

    #[must_use]
    pub const fn canonical() -> Self {
        Self {
            max_parent_edges: 64,
            max_addressed_recipients: 10_000,
            max_explicit_audience_members: 10_000,
            max_resolved_group_references: 64,
            max_delivery_attempts_per_recipient: 256,
            max_operation_output_slots: 256,
            max_inline_body_bytes: 65_536,
            max_external_resource_bytes: Self::ONE_TEBIBYTE,
            max_membership_proof_siblings: 64,
            max_membership_proof_bytes: 8_192,
        }
    }
}

impl Default for InformationLimitsV1 {
    fn default() -> Self {
        Self::canonical()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCapability {
    PersistentInstance,
    NonexclusiveAccess,
    SimultaneousAccess,
    AddressedDelivery,
    AudienceDelivery,
    OpenReception,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChannelPayload {
    pub profile: String,
    pub capabilities: Vec<ChannelCapability>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestAlgorithm {
    Sha256,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ContentDigest {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InformationBody {
    InlineJson {
        value: Value,
    },
    Resource {
        digest: ContentDigest,
        media_type: String,
        byte_length: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentSourceRole {
    Contribution,
    Quotation,
    Correction,
    Contradiction,
    Context,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ContentSourceEdge {
    pub source: TypedDomainRecordRef<Content>,
    pub role: ContentSourceRole,
    pub completeness_per_mille: u16,
    pub fidelity_per_mille: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentDerivation {
    pub operation: String,
    pub sources: Vec<ContentSourceEdge>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContentPayload {
    pub content_type: String,
    pub body: InformationBody,
    pub created_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation: Option<ContentDerivation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentRelation {
    SameContent,
    DerivedContent,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RepresentationSourceEdge {
    pub parent: TypedDomainRecordRef<Representation>,
    pub completeness_per_mille: u16,
    pub fidelity_per_mille: u16,
}

/// A semantic source attribution carried by a representation.
///
/// This is deliberately a namespaced claim identifier rather than an engine
/// entity or audit reference. It records what the representation says about
/// its source without asserting that the claim is true or exposing the actor
/// and operation that actually produced the record.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ClaimedSourceRef {
    pub namespace: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepresentationPayload {
    pub format: String,
    pub created_at: SimTime,
    pub operation: String,
    pub content_relation: ContentRelation,
    pub sources: Vec<RepresentationSourceEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_source: Option<ClaimedSourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation_capability: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Available,
    Unavailable,
    LocationUnknown,
    Destroyed,
    Consumed,
}

impl InstanceStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Destroyed | Self::Consumed)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstancePayload {
    pub created_at: SimTime,
    pub status: InstanceStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStatus {
    Prepared,
    Active,
    Completed,
    Cancelled,
}

impl DispatchStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DispatchTarget {
    Addressed(Vec<KnowledgeHolderRef>),
    Audience(TypedDomainRecordRef<Audience>),
    Open,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DispatchPayload {
    pub status: DispatchStatus,
    pub target: DispatchTarget,
    pub prepared_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<SimTime>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAttemptStatus {
    Prepared,
    InTransit,
    Delivered,
    Failed,
    Cancelled,
}

impl DeliveryAttemptStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Delivered | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliveryAttemptPayload {
    pub status: DeliveryAttemptStatus,
    pub attempt_number: u32,
    pub prepared_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<SimTime>,
    pub due_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<SimTime>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccessPayload {
    pub accessed_at: SimTime,
    pub method: String,
    pub extent_per_mille: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterpretationStatus {
    Failed,
    Partial,
    Succeeded,
}

/// Longest [`AuthenticityFinding::basis`] the lifecycle admits, in bytes.
pub const MAX_AUTHENTICITY_BASIS_BYTES: usize = 256;

/// The interpreting holder's judgement of a representation's claimed source.
///
/// The finding binds to one exact version of an interpreted representation
/// that carries a [`ClaimedSourceRef`]. It records whether the interpreter
/// accepts that claim, on what basis, and with what confidence; it does not
/// state who actually produced the representation, which stays protected
/// audit origin. How likely a holder is to detect a false claim is an
/// application draw.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticityFinding {
    /// Exact version of the interpreted representation the finding judges.
    pub representation: DomainRecordVersionRef,
    pub claimed_source_accepted: bool,
    /// Canonical application code or short explanation, at most
    /// [`MAX_AUTHENTICITY_BASIS_BYTES`] bytes.
    pub basis: String,
    pub confidence_per_mille: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InterpretationPayload {
    pub interpreted_at: SimTime,
    pub status: InterpretationStatus,
    pub capability: String,
    pub confidence_per_mille: u16,
    /// Authenticity finding written by the interpreting holder's
    /// interpretation. Omitted from JSON when absent, so interpretations
    /// without a finding keep their exact encoding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity: Option<AuthenticityFinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InterpretationAuthority {
    HolderSelf,
    InstitutionalRole {
        assignment: DomainRecordVersionRef,
        authority_grant: String,
    },
    Delegated {
        evidence: EvidenceRef,
        authority_grant: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DelegationEvidenceSelector {
    Command {
        producer_plugin: String,
        command_type: String,
    },
    Ingress {
        producer_plugin: String,
        packet_type: String,
    },
    DomainRecord {
        owner_plugin: String,
        kind: DomainRecordKind,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DelegationAuthorityGrant {
    pub code: String,
    pub selector: DelegationEvidenceSelector,
    pub claim_path: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationClaimV1 {
    pub format_version: u32,
    pub performed_by: EntityRef,
    pub performed_for: KnowledgeHolderRef,
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<SimTime>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityAssignmentPayload {
    pub claim: DelegationClaimV1,
}

/// A delegation claim the information plugin recorded when the command that
/// grants it was admitted: one entry of a [`DelegatedAuthorityLedgerPayload`].
///
/// A [`DelegationEvidenceSelector::Command`] grant names commands of another
/// plugin. When the information plugin's lifecycle system sees such a
/// command admitted at a boundary, it stores the claim found at the grant's
/// claim path in the grant's ledger, keyed by the command ID. An
/// [`InterpretationAuthority::Delegated`] interpretation that cites the
/// command is checked against this entry, never against the command
/// itself, so the check does not depend on whether a seal has removed the
/// command's payload. A command whose payload has no decodable claim at the
/// claim path, or whose claim has already expired, is not recorded, and
/// citing it is refused as before.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedAuthorityPayload {
    /// Code of the [`DelegationAuthorityGrant`] whose selector matched.
    pub authority_grant: String,
    /// The admitted command that granted the delegation.
    pub command: CommandId,
    /// The claim found at the grant's claim path, as the command carried it.
    pub claim: DelegationClaimV1,
}

/// Wire version of [`DelegatedAuthorityLedgerPayload`].
pub const DELEGATED_AUTHORITY_LEDGER_FORMAT_VERSION: u32 = 1;

/// The delegations of one command grant that are still in force, in one
/// record per grant (see [`crate::delegated_authority_reference`]).
///
/// A delegation stays while its claim can still authorize: the lifecycle
/// system, at every boundary it runs, removes each delegation whose
/// `expires_at` is at or before the boundary time, found through
/// [`Self::expiring`] without scanning the ledger, and a delegation whose
/// `expires_at` has come authorizes nothing even before it is removed. So
/// the ledger holds the live delegations only; a claim with no `expires_at`
/// stays for the run. A removed delegation leaves nothing in state: the
/// interpretations that relied on it keep the hash of its claim
/// (`authority_claim_hash` of their operation), and its command stays in the
/// evidence journal until a seal removes it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedAuthorityLedgerPayload {
    pub format_version: u32,
    /// Code of the command grant whose delegations these are.
    pub authority_grant: String,
    /// Live delegations, by ascending command ID.
    pub delegations: Vec<DelegatedAuthorityPayload>,
    /// The live delegations that expire, by ascending expiry time and then
    /// command ID; the first is the next to be removed.
    pub expiring: Vec<DelegationExpiry>,
}

/// When one live delegation expires.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationExpiry {
    pub expires_at: SimTime,
    pub command: CommandId,
}

impl DelegatedAuthorityLedgerPayload {
    /// An empty ledger for `authority_grant`.
    #[must_use]
    pub fn new(authority_grant: impl Into<String>) -> Self {
        Self {
            format_version: DELEGATED_AUTHORITY_LEDGER_FORMAT_VERSION,
            authority_grant: authority_grant.into(),
            delegations: Vec::new(),
            expiring: Vec::new(),
        }
    }

    /// The live delegation `command` granted, if it is recorded.
    #[must_use]
    pub fn delegation(&self, command: CommandId) -> Option<&DelegatedAuthorityPayload> {
        self.delegations
            .binary_search_by_key(&command, |delegation| delegation.command)
            .ok()
            .map(|index| &self.delegations[index])
    }

    /// Records a delegation; returns `false` when its command is already
    /// recorded.
    pub fn insert(&mut self, delegation: DelegatedAuthorityPayload) -> bool {
        let Err(index) = self
            .delegations
            .binary_search_by_key(&delegation.command, |recorded| recorded.command)
        else {
            return false;
        };
        if let Some(expires_at) = delegation.claim.expires_at {
            let expiry = DelegationExpiry {
                expires_at,
                command: delegation.command,
            };
            let position = self.expiring.partition_point(|entry| *entry < expiry);
            self.expiring.insert(position, expiry);
        }
        self.delegations.insert(index, delegation);
        true
    }

    /// Removes every delegation whose `expires_at` is at or before `at`;
    /// returns how many it removed.
    pub fn retire_expired(&mut self, at: SimTime) -> usize {
        let due = self
            .expiring
            .partition_point(|entry| entry.expires_at <= at);
        for expiry in self.expiring.drain(..due) {
            if let Ok(index) = self
                .delegations
                .binary_search_by_key(&expiry.command, |delegation| delegation.command)
            {
                self.delegations.remove(index);
            }
        }
        due
    }

    /// Checks the ledger's own invariants: sorted unique delegations of this
    /// grant, and one expiry entry for each delegation that expires.
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != DELEGATED_AUTHORITY_LEDGER_FORMAT_VERSION
            || self
                .delegations
                .windows(2)
                .any(|pair| pair[0].command >= pair[1].command)
            || self
                .delegations
                .iter()
                .any(|delegation| delegation.authority_grant != self.authority_grant)
            || self.expiring.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("delegation ledger is not canonical".to_owned());
        }
        let mut expected = self
            .delegations
            .iter()
            .filter_map(|delegation| {
                delegation
                    .claim
                    .expires_at
                    .map(|expires_at| DelegationExpiry {
                        expires_at,
                        command: delegation.command,
                    })
            })
            .collect::<Vec<_>>();
        expected.sort();
        if expected != self.expiring {
            return Err("delegation ledger expiry index does not match its delegations".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudienceMembership {
    ExplicitMembers,
    ResolvedGroupSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudiencePayload {
    pub membership: AudienceMembership,
    pub resolved_at: SimTime,
    pub resolution_version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_boundary: Option<BoundaryId>,
    pub member_count: u64,
    pub membership_root: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudienceMembershipLeafV1 {
    pub format_version: u32,
    pub holder: KnowledgeHolderRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AudienceMembershipProofV1 {
    pub holder: KnowledgeHolderRef,
    pub leaf_index: u64,
    pub member_count: u64,
    pub sibling_hashes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AudienceAccessEvidence {
    ListedMember,
    MembershipProof(AudienceMembershipProofV1),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    Prepared,
    Active,
    Withdrawn,
    Expired,
}

impl ReleaseStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Withdrawn | Self::Expired)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseScope {
    Audience,
    OpenAvailability,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleasePayload {
    pub status: ReleaseStatus,
    pub scope: ReleaseScope,
    pub prepared_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_at: Option<SimTime>,
}

/// Cross-record bindings used by the access validator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccessContext {
    pub representation: TypedDomainRecordRef<Representation>,
    pub instance: Option<TypedDomainRecordRef<Instance>>,
    pub dispatch: Option<TypedDomainRecordRef<Dispatch>>,
    pub delivery_attempt: Option<TypedDomainRecordRef<DeliveryAttempt>>,
    pub release: Option<TypedDomainRecordRef<Release>>,
}
