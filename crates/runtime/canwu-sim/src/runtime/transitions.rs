//! Transition manifests and the omitted-participant audit.
//!
//! A coordinating plugin registers a [`TransitionManifest`] that names every
//! plugin whose writes form one multi-owner transition, together with the
//! record versions each participant expects before and after it. At the
//! manifest's `ready_at` boundary, each listed participant stages its part in
//! phase 10 through [`BoundaryDirective::StageTransitionWrite`]. Before the
//! phase-11 conditional-transition commit, the kernel audits every manifest
//! that is ready at the boundary:
//!
//! - when every participant staged, all `expected_pre` versions must be the
//!   current versions, and after the same-boundary commit every
//!   `expected_post` version must be the version the boundary's phase-7 and
//!   phase-10 writes produce; the manifest settles as
//!   [`TransitionAuditOutcome::Committed`];
//! - when no participant staged, the manifest settles as
//!   [`TransitionAuditOutcome::Expired`] and nothing is written for it;
//! - when some, but not all, participants staged, or a version differs, the
//!   whole boundary fails closed and rolls back
//!   ([`ErrorCode::TransitionParticipantMissing`],
//!   [`ErrorCode::TransitionVersionMismatch`]).
//!
//! Because a manifest whose participants are all silent expires rather than
//! fails, omission is detected only for a manifest with two or more
//! participants; a single-participant manifest cannot fail for omission.
//! Expiry is also the recovery from a boundary that fails on every retry: a
//! ready boundary in which no participant stages, for example one settled
//! with a cadence under which no participant system runs, expires the
//! manifest, and the coordinator may register a new attempt.
//!
//! A manifest never spans two boundaries: it may be registered up to
//! [`MAX_TRANSITION_READY_HORIZON`] boundaries ahead, but its writes stage and
//! commit in its single ready boundary. Pending manifests are persisted under
//! the scheduler commitment, and audit records are hash-chained boundary
//! evidence visible to the coordinator and participants in phase 11 and later
//! of the settling boundary.

use super::{
    BoundaryDirective, BoundaryId, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource, EntityRef,
    ErrorCode, PluginRegistry, RuntimeState, Simulation, SimulationSnapshot, StateKey,
    canonical_text, invalid_snapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

/// Maximum number of registered manifests awaiting their ready boundary,
/// across all coordinators. Manifests that settle in a boundary still count
/// until its phase 11.
pub const MAX_PENDING_TRANSITION_MANIFESTS: usize = 128;
/// Maximum number of pending manifests one coordinator may hold, so a single
/// coordinator cannot exhaust [`MAX_PENDING_TRANSITION_MANIFESTS`].
pub const MAX_PENDING_TRANSITION_MANIFESTS_PER_COORDINATOR: usize = 32;
/// Maximum number of boundaries between a registration and its `ready_at`
/// boundary, so every pending manifest settles within a bounded number of
/// boundaries and no lineage stays locked indefinitely.
pub const MAX_TRANSITION_READY_HORIZON: u64 = 1_024;
/// Maximum number of participants one manifest may list.
pub const MAX_TRANSITION_PARTICIPANTS: usize = 16;
/// Maximum number of `expected_pre` and `expected_post` entries, together,
/// across all participants of one manifest. With the pending bounds, this
/// bounds the number of entries in the pending set; each record reference
/// names a registered record kind and follows the ordinary record-ID rules.
pub const MAX_TRANSITION_EXPECTED_VERSIONS: usize = 64;
/// Maximum byte length of a manifest lineage ID.
pub const MAX_TRANSITION_LINEAGE_ID_BYTES: usize = 256;

const REGISTRATION_PHASES: [BoundaryPhase; 3] = [
    BoundaryPhase::DomainDeltaProposal,
    BoundaryPhase::HistoricalCandidateEvaluation,
    BoundaryPhase::StrategicAggregation,
];

/// Stable identity of a registered manifest. The coordinator is the
/// registering plugin, recorded by the kernel rather than claimed by content.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct TransitionManifestId {
    pub coordinator: String,
    pub lineage_id: String,
    pub attempt: u32,
}

impl Display for TransitionManifestId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}/{}#{}",
            self.coordinator, self.lineage_id, self.attempt
        )
    }
}

/// A record version a participant expects the transition to produce.
///
/// Unlike [`DomainRecordVersionRef`], it carries no establishing change,
/// because that change does not exist until the transition commits.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct TransitionRecordVersion {
    pub record: DomainRecordRef,
    pub version: u64,
}

/// One plugin whose writes a transition requires.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionParticipant {
    pub plugin: String,
    /// Exact current versions, including their establishing change, that the
    /// committed state read by phase 10 must hold when the transition settles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_pre: Vec<DomainRecordVersionRef>,
    /// Versions these records must hold as of phase 11: after the boundary's
    /// same-boundary phase-10 writes commit and with its pending next-boundary
    /// phase-7 and phase-10 writes applied. Phase-12 and phase-13 writes of
    /// the same boundary may still change a record after this check.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_post: Vec<TransitionRecordVersion>,
}

impl TransitionParticipant {
    #[must_use]
    pub fn new(plugin: impl Into<String>) -> Self {
        Self {
            plugin: plugin.into(),
            expected_pre: Vec::new(),
            expected_post: Vec::new(),
        }
    }
}

/// Declaration of the participants of one multi-owner transition and the
/// single boundary in which all of their writes stage and commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionManifest {
    pub lineage_id: String,
    pub attempt: u32,
    pub participants: Vec<TransitionParticipant>,
    pub ready_at: BoundaryId,
}

/// A registered manifest awaiting its ready boundary, and the registration
/// evidence recorded on the boundary that admitted it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingTransitionManifest {
    /// Registering plugin.
    pub coordinator: String,
    /// Registering boundary system.
    pub system: String,
    pub registered_at: BoundaryId,
    pub manifest: TransitionManifest,
}

impl PendingTransitionManifest {
    #[must_use]
    pub fn id(&self) -> TransitionManifestId {
        TransitionManifestId {
            coordinator: self.coordinator.clone(),
            lineage_id: self.manifest.lineage_id.clone(),
            attempt: self.manifest.attempt,
        }
    }

    /// Returns whether `plugin` is a listed participant.
    #[must_use]
    pub fn lists(&self, plugin: &str) -> bool {
        self.manifest
            .participants
            .iter()
            .any(|participant| participant.plugin == plugin)
    }

    /// Returns whether `plugin` coordinates or participates in the manifest.
    pub(super) fn involves(&self, plugin: &str) -> bool {
        self.coordinator == plugin || self.lists(plugin)
    }
}

/// Terminal outcome of a manifest at its ready boundary. Failed checks leave
/// no record: they fail the boundary instead.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionAuditOutcome {
    /// Every participant staged and every expected version held.
    Committed,
    /// The ready boundary settled without any participant staging, so
    /// nothing was written for the manifest. Silence of every participant
    /// expires a manifest instead of failing the boundary; only a manifest
    /// that some, but not all, participants staged fails for omission.
    Expired,
}

/// How many writes one participant staged for a manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionParticipantAudit {
    pub plugin: String,
    pub staged_writes: u64,
}

/// Read-only audit evidence for one manifest settled at its ready boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionAuditRecord {
    pub manifest_id: TransitionManifestId,
    /// The settling boundary. A manifest ID may be registered again after it
    /// settles, but a lineage has at most one pending manifest and its
    /// successor is ready at a later boundary, so the ID together with
    /// `ready_at` identifies an audit uniquely over the run's history.
    pub ready_at: BoundaryId,
    pub outcome: TransitionAuditOutcome,
    /// Participants in manifest order.
    pub participants: Vec<TransitionParticipantAudit>,
}

impl TransitionAuditRecord {
    /// Returns whether `plugin` coordinated or participated in the manifest.
    pub(super) fn involves(&self, plugin: &str) -> bool {
        self.manifest_id.coordinator == plugin
            || self
                .participants
                .iter()
                .any(|participant| participant.plugin == plugin)
    }
}

/// Registrations and audits a committed boundary produced, and the pending
/// set that follows it.
pub(super) struct BoundaryTransitionEvidence {
    pub(super) registered: Vec<PendingTransitionManifest>,
    pub(super) audits: Vec<TransitionAuditRecord>,
    pub(super) pending: BTreeMap<TransitionManifestId, PendingTransitionManifest>,
}

/// Boundary-local transition state. It starts from the committed pending
/// manifests and is written back only when the boundary commits.
pub(super) struct BoundaryTransitionLedger {
    boundary: BoundaryId,
    pending: BTreeMap<TransitionManifestId, PendingTransitionManifest>,
    proposed: Vec<PendingTransitionManifest>,
    registered: Vec<PendingTransitionManifest>,
    staged: BTreeMap<TransitionManifestId, BTreeMap<String, u64>>,
    audits: Vec<TransitionAuditRecord>,
    post_checks: Vec<(TransitionManifestId, TransitionRecordVersion)>,
}

const fn is_transition_directive(directive: &BoundaryDirective) -> bool {
    matches!(
        directive,
        BoundaryDirective::RegisterTransitionManifest { .. }
            | BoundaryDirective::StageTransitionWrite { .. }
    )
}

fn require_transition_write(
    plugin: &str,
    contract: &BoundarySystemContract,
    phases: &[BoundaryPhase],
    directive: &str,
) -> Result<(), CanwuError> {
    let key = StateKey::core_transitions();
    if !phases.contains(&contract.phase) || !contract.writes.contains(&key) {
        return Err(CanwuError::new(
            ErrorCode::UndeclaredStateWrite,
            format!(
                "boundary system {plugin}.{} cannot propose {directive}: it must run in phase {} and declare core write {}.{}",
                contract.name,
                phases
                    .iter()
                    .map(|phase| (*phase as u8).to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                key.namespace,
                key.name
            ),
        ));
    }
    Ok(())
}

/// Returns whether a plugin has a phase-10 system that may stage transition
/// writes.
fn plugin_can_stage(plugins: &PluginRegistry, plugin: &str) -> bool {
    let key = StateKey::core_transitions();
    plugins.descriptors.get(plugin).is_some_and(|descriptor| {
        descriptor.boundary_systems.iter().any(|contract| {
            contract.phase == BoundaryPhase::HistoricalCandidateEvaluation
                && contract.writes.contains(&key)
        })
    })
}

fn invalid_manifest(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidBoundary, message)
}

/// Structural manifest rules shared by admission and snapshot validation.
fn validate_manifest(
    manifest: &TransitionManifest,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    if !canonical_text(&manifest.lineage_id)
        || manifest.lineage_id.len() > MAX_TRANSITION_LINEAGE_ID_BYTES
    {
        return Err(invalid_manifest(format!(
            "transition manifest lineage IDs must be canonical text of at most {MAX_TRANSITION_LINEAGE_ID_BYTES} bytes"
        )));
    }
    if manifest.participants.is_empty() || manifest.participants.len() > MAX_TRANSITION_PARTICIPANTS
    {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            format!(
                "transition manifest {} must list between 1 and {MAX_TRANSITION_PARTICIPANTS} participants",
                manifest.lineage_id
            ),
        ));
    }
    let expected_versions = manifest
        .participants
        .iter()
        .map(|participant| participant.expected_pre.len() + participant.expected_post.len())
        .sum::<usize>();
    if expected_versions > MAX_TRANSITION_EXPECTED_VERSIONS {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            format!(
                "transition manifest {} declares more than {MAX_TRANSITION_EXPECTED_VERSIONS} expected versions",
                manifest.lineage_id
            ),
        ));
    }
    let mut plugin_names = BTreeSet::new();
    for participant in &manifest.participants {
        if !canonical_text(&participant.plugin) || !plugin_names.insert(&participant.plugin) {
            return Err(invalid_manifest(format!(
                "transition manifest {} must list unique canonical participant plugins",
                manifest.lineage_id
            )));
        }
        if !plugin_can_stage(plugins, &participant.plugin) {
            return Err(invalid_manifest(format!(
                "transition participant {} has no phase-10 system that declares core write {}.{}",
                participant.plugin,
                StateKey::core_transitions().namespace,
                StateKey::core_transitions().name
            )));
        }
        let valid_record = |record: &DomainRecordRef| {
            plugins.record_schemas.contains_key(&record.kind) && canonical_text(&record.id)
        };
        let mut pre = BTreeSet::new();
        let mut post = BTreeSet::new();
        if participant.expected_pre.iter().any(|expected| {
            expected.version == 0
                || !valid_record(&expected.record)
                || !pre.insert(&expected.record)
        }) || participant.expected_post.iter().any(|expected| {
            expected.version == 0
                || !valid_record(&expected.record)
                || !post.insert(&expected.record)
        }) {
            return Err(invalid_manifest(format!(
                "transition participant {} must name each expected record of a registered kind once, with a canonical ID and a nonzero version",
                participant.plugin
            )));
        }
    }
    Ok(())
}

/// A manifest registered in phase 7 may be ready in the same boundary;
/// registrations in later phases must name a later boundary. No manifest may
/// be ready more than [`MAX_TRANSITION_READY_HORIZON`] boundaries ahead.
fn ready_at_is_valid(
    phase: BoundaryPhase,
    registered_at: BoundaryId,
    ready_at: BoundaryId,
) -> bool {
    let earliest = if phase == BoundaryPhase::DomainDeltaProposal {
        registered_at.get()
    } else {
        registered_at.get().saturating_add(1)
    };
    ready_at.get() >= earliest
        && ready_at.get() - registered_at.get() <= MAX_TRANSITION_READY_HORIZON
}

/// Checks the global and per-coordinator pending bounds and the single
/// pending manifest per lineage, shared by admission and snapshot validation.
fn pending_admission_error<'a>(
    pending: impl Iterator<Item = &'a PendingTransitionManifest>,
    coordinator: &str,
    lineage_id: &str,
) -> Option<CanwuError> {
    let mut total = 0;
    let mut own = 0;
    for existing in pending {
        total += 1;
        if existing.coordinator == coordinator {
            own += 1;
            if existing.manifest.lineage_id == lineage_id {
                return Some(invalid_manifest(format!(
                    "coordinator {coordinator} already has a pending transition manifest for lineage {lineage_id}"
                )));
            }
        }
    }
    if total >= MAX_PENDING_TRANSITION_MANIFESTS
        || own >= MAX_PENDING_TRANSITION_MANIFESTS_PER_COORDINATOR
    {
        return Some(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            format!(
                "at most {MAX_PENDING_TRANSITION_MANIFESTS} transition manifests, and {MAX_PENDING_TRANSITION_MANIFESTS_PER_COORDINATOR} per coordinator, may be pending"
            ),
        ));
    }
    None
}

fn version_list(values: &[String]) -> String {
    values.join(", ")
}

fn version_source(source: &DomainRecordVersionSource) -> String {
    match source {
        DomainRecordVersionSource::InitialScenario => "the initial scenario".to_owned(),
        DomainRecordVersionSource::BoundaryChange {
            boundary,
            change_index,
        } => format!("boundary {boundary} change {change_index}"),
    }
}

impl BoundaryTransitionLedger {
    pub(super) fn new(
        boundary: BoundaryId,
        pending: &BTreeMap<TransitionManifestId, PendingTransitionManifest>,
    ) -> Self {
        Self {
            boundary,
            pending: pending.clone(),
            proposed: Vec::new(),
            registered: Vec::new(),
            staged: BTreeMap::new(),
            audits: Vec::new(),
            post_checks: Vec::new(),
        }
    }

    /// Pending manifests visible at this cut, in manifest-ID order.
    pub(super) fn pending(&self) -> impl Iterator<Item = &PendingTransitionManifest> {
        self.pending.values()
    }

    /// Audit records settled so far in this boundary, in manifest-ID order.
    pub(super) fn audits(&self) -> &[TransitionAuditRecord] {
        &self.audits
    }

    /// Admits the transition directives of one system proposal and returns
    /// the proposal with registrations removed and staged writes lifted into
    /// ordinary directives, which then follow every ordinary rule.
    pub(super) fn admit(
        &mut self,
        plugin: &str,
        contract: &BoundarySystemContract,
        plugins: &PluginRegistry,
        mut proposal: BoundaryProposal,
    ) -> Result<BoundaryProposal, CanwuError> {
        if !proposal.directives.iter().any(is_transition_directive) {
            return Ok(proposal);
        }
        let directives = std::mem::take(&mut proposal.directives);
        let mut flattened = Vec::with_capacity(directives.len());
        for directive in directives {
            match directive {
                BoundaryDirective::RegisterTransitionManifest { manifest } => {
                    self.admit_registration(plugin, contract, plugins, manifest)?;
                }
                BoundaryDirective::StageTransitionWrite {
                    manifest_id,
                    writes,
                } => {
                    self.admit_stage(plugin, contract, &manifest_id, &writes)?;
                    flattened.extend(writes);
                }
                directive => flattened.push(directive),
            }
        }
        proposal.directives = flattened;
        Ok(proposal)
    }

    fn admit_registration(
        &mut self,
        plugin: &str,
        contract: &BoundarySystemContract,
        plugins: &PluginRegistry,
        manifest: TransitionManifest,
    ) -> Result<(), CanwuError> {
        require_transition_write(
            plugin,
            contract,
            &REGISTRATION_PHASES,
            "RegisterTransitionManifest",
        )?;
        validate_manifest(&manifest, plugins)?;
        if !ready_at_is_valid(contract.phase, self.boundary, manifest.ready_at) {
            return Err(invalid_manifest(format!(
                "transition manifest {} registered in phase {} of boundary {} cannot be ready at boundary {}; it must be ready within {MAX_TRANSITION_READY_HORIZON} boundaries",
                manifest.lineage_id, contract.phase as u8, self.boundary, manifest.ready_at
            )));
        }
        if let Some(error) = pending_admission_error(
            self.pending.values().chain(&self.proposed),
            plugin,
            &manifest.lineage_id,
        ) {
            return Err(error);
        }
        self.proposed.push(PendingTransitionManifest {
            coordinator: plugin.to_owned(),
            system: contract.name.clone(),
            registered_at: self.boundary,
            manifest,
        });
        Ok(())
    }

    fn admit_stage(
        &mut self,
        plugin: &str,
        contract: &BoundarySystemContract,
        manifest_id: &TransitionManifestId,
        writes: &[BoundaryDirective],
    ) -> Result<(), CanwuError> {
        require_transition_write(
            plugin,
            contract,
            &[BoundaryPhase::HistoricalCandidateEvaluation],
            "StageTransitionWrite",
        )?;
        let Some(pending) = self
            .pending
            .get(manifest_id)
            .filter(|pending| pending.manifest.ready_at == self.boundary)
        else {
            return Err(invalid_manifest(format!(
                "transition manifest {manifest_id} is not pending and ready at boundary {}",
                self.boundary
            )));
        };
        if !pending.lists(plugin) {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "plugin {plugin} is not a participant of transition manifest {manifest_id}"
                ),
            ));
        }
        if writes.iter().any(is_transition_directive) {
            return Err(invalid_manifest(
                "staged transition writes cannot register or stage transition manifests",
            ));
        }
        let count = self
            .staged
            .entry(manifest_id.clone())
            .or_default()
            .entry(plugin.to_owned())
            .or_default();
        *count = count
            .checked_add(u64::try_from(writes.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    "staged transition write count is exhausted",
                )
            })?;
        Ok(())
    }

    /// Makes the registrations of a completed phase visible to later phases.
    pub(super) fn close_phase(&mut self) {
        for manifest in self.proposed.drain(..) {
            self.pending.insert(manifest.id(), manifest.clone());
            self.registered.push(manifest);
        }
    }

    /// Phase-11 audit before the transition bundle commits. Settles every
    /// manifest ready at this boundary, or fails closed.
    pub(super) fn settle_ready(&mut self, state: &RuntimeState) -> Result<(), CanwuError> {
        if let Some(overdue) = self
            .pending
            .values()
            .find(|pending| pending.manifest.ready_at < self.boundary)
        {
            return invalid_snapshot(format!(
                "transition manifest {} passed its ready boundary without an audit",
                overdue.id()
            ));
        }
        let ready: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.manifest.ready_at == self.boundary)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ready {
            let Some(pending) = self.pending.remove(&id) else {
                continue;
            };
            let staged = self.staged.remove(&id).unwrap_or_default();
            let participants: Vec<_> = pending
                .manifest
                .participants
                .iter()
                .map(|participant| TransitionParticipantAudit {
                    plugin: participant.plugin.clone(),
                    staged_writes: staged.get(&participant.plugin).copied().unwrap_or(0),
                })
                .collect();
            let missing: Vec<_> = pending
                .manifest
                .participants
                .iter()
                .filter(|participant| !staged.contains_key(&participant.plugin))
                .map(|participant| participant.plugin.clone())
                .collect();
            let outcome = if missing.len() == pending.manifest.participants.len() {
                TransitionAuditOutcome::Expired
            } else if !missing.is_empty() {
                return Err(CanwuError::new(
                    ErrorCode::TransitionParticipantMissing,
                    format!(
                        "transition manifest {id} at boundary {} is missing participants: {}",
                        self.boundary,
                        version_list(&missing)
                    ),
                ));
            } else {
                let mut mismatched = Vec::new();
                let mut related = Vec::new();
                for participant in &pending.manifest.participants {
                    for expected in &participant.expected_pre {
                        let current =
                            super::current_domain_record_version(state, &expected.record)?;
                        if current.as_ref() != Some(expected) {
                            mismatched.push(format!(
                                "{} expected version {} from {} but found {}",
                                expected.record,
                                expected.version,
                                version_source(&expected.established_by),
                                current.map_or_else(
                                    || "no record".to_owned(),
                                    |current| format!(
                                        "version {} from {}",
                                        current.version,
                                        version_source(&current.established_by)
                                    )
                                )
                            ));
                            related.push(EntityRef::Domain(expected.record.clone()));
                        }
                    }
                }
                if !mismatched.is_empty() {
                    return Err(version_mismatch(&id, "pre", &mismatched, related));
                }
                self.post_checks
                    .extend(
                        pending
                            .manifest
                            .participants
                            .iter()
                            .flat_map(|participant| {
                                participant
                                    .expected_post
                                    .iter()
                                    .map(|expected| (id.clone(), expected.clone()))
                            }),
                    );
                TransitionAuditOutcome::Committed
            };
            self.audits.push(TransitionAuditRecord {
                manifest_id: id,
                ready_at: self.boundary,
                outcome,
                participants,
            });
        }
        if let Some((id, _)) = self.staged.iter().next() {
            return invalid_snapshot(format!(
                "transition writes were staged for manifest {id}, which is not ready"
            ));
        }
        Ok(())
    }

    /// Returns whether a committed manifest of this boundary declared
    /// `expected_post` versions.
    pub(super) fn requires_post_check(&self) -> bool {
        !self.post_checks.is_empty()
    }

    /// Checks the `expected_post` versions of the manifests committed in this
    /// boundary against the versions its phase-10 writes produce.
    pub(super) fn check_expected_post(
        &mut self,
        version_of: &dyn Fn(&DomainRecordRef) -> Option<u64>,
    ) -> Result<(), CanwuError> {
        let mut failures: BTreeMap<TransitionManifestId, (Vec<String>, Vec<EntityRef>)> =
            BTreeMap::new();
        for (id, expected) in std::mem::take(&mut self.post_checks) {
            let actual = version_of(&expected.record);
            if actual != Some(expected.version) {
                let entry = failures.entry(id).or_default();
                entry.0.push(format!(
                    "{} expected version {} but found {}",
                    expected.record,
                    expected.version,
                    actual.map_or_else(
                        || "no record".to_owned(),
                        |version| format!("version {version}")
                    )
                ));
                entry.1.push(EntityRef::Domain(expected.record));
            }
        }
        if let Some((id, (mismatched, related))) = failures.into_iter().next() {
            return Err(version_mismatch(&id, "post", &mismatched, related));
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> BoundaryTransitionEvidence {
        self.close_phase();
        BoundaryTransitionEvidence {
            registered: self.registered,
            audits: self.audits,
            pending: self.pending,
        }
    }
}

fn version_mismatch(
    id: &TransitionManifestId,
    stage: &str,
    mismatched: &[String],
    related: Vec<EntityRef>,
) -> CanwuError {
    let mut error = CanwuError::new(
        ErrorCode::TransitionVersionMismatch,
        format!(
            "transition manifest {id} has mismatched expected_{stage} versions: {}",
            version_list(mismatched)
        ),
    );
    error.related_entities = related;
    error
}

impl Simulation {
    /// Registered transition manifests whose ready boundary has not settled,
    /// in manifest-ID order.
    pub fn pending_transition_manifests(&self) -> impl Iterator<Item = &PendingTransitionManifest> {
        self.state.scheduler.transition_manifests.values()
    }
}

fn insert_snapshot_pending(
    pending: &mut BTreeMap<TransitionManifestId, PendingTransitionManifest>,
    registered: &PendingTransitionManifest,
) -> Result<(), CanwuError> {
    if pending_admission_error(
        pending.values(),
        &registered.coordinator,
        &registered.manifest.lineage_id,
    )
    .is_some()
    {
        return invalid_snapshot(
            "transition manifest registrations exceed the pending bounds or repeat a pending lineage",
        );
    }
    pending.insert(registered.id(), registered.clone());
    Ok(())
}

/// Validates committed registration and audit evidence and proves that it
/// reconstructs the persisted pending manifests.
///
/// The participant and version checks themselves read live staging, so they
/// are proven by exact replay; this pass checks the manifest lifecycle: each
/// registration has a due declared writer and a valid shape, each audit settles
/// exactly the manifests ready at its boundary, and no manifest outlives its
/// ready boundary.
pub(super) fn validate_snapshot_transitions(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    let key = StateKey::core_transitions();
    let mut pending: BTreeMap<TransitionManifestId, PendingTransitionManifest> = BTreeMap::new();
    for record in &snapshot.boundaries {
        let mut previous_order = None;
        let mut late = Vec::new();
        for registered in &record.transition_manifests {
            let Some(contract) = super::validation::snapshot_boundary_contract(
                plugins,
                &registered.coordinator,
                &registered.system,
            )
            .filter(|contract| {
                contract.writes.contains(&key)
                    && REGISTRATION_PHASES.contains(&contract.phase)
                    && super::boundary_system_due(
                        contract,
                        &record.cadences,
                        super::boundary_has_event_ingress(record),
                    )
            }) else {
                return invalid_snapshot("transition manifest registration has no declared writer");
            };
            let order = (
                contract.phase,
                registered.coordinator.as_str(),
                registered.system.as_str(),
            );
            if registered.registered_at != record.id
                || previous_order.is_some_and(|previous| previous > order)
                || !ready_at_is_valid(contract.phase, record.id, registered.manifest.ready_at)
                || validate_manifest(&registered.manifest, plugins).is_err()
            {
                return invalid_snapshot(
                    "transition manifest registration evidence is inconsistent",
                );
            }
            previous_order = Some(order);
            if contract.phase < BoundaryPhase::ConditionalTransitionCommit {
                insert_snapshot_pending(&mut pending, registered)?;
            } else {
                late.push(registered);
            }
        }
        let ready: Vec<_> = pending
            .iter()
            .filter(|(_, pending)| pending.manifest.ready_at == record.id)
            .map(|(id, _)| id.clone())
            .collect();
        if ready.len() != record.transition_audits.len()
            || ready
                .iter()
                .zip(&record.transition_audits)
                .any(|(id, audit)| *id != audit.manifest_id)
        {
            return invalid_snapshot(
                "transition audits do not settle exactly the manifests ready at their boundary",
            );
        }
        for audit in &record.transition_audits {
            let Some(settled) = pending.remove(&audit.manifest_id) else {
                return invalid_snapshot("transition audit names no pending manifest");
            };
            if audit.ready_at != record.id
                || audit.participants.len() != settled.manifest.participants.len()
                || audit
                    .participants
                    .iter()
                    .zip(&settled.manifest.participants)
                    .any(|(audited, listed)| audited.plugin != listed.plugin)
                || (audit.outcome == TransitionAuditOutcome::Expired
                    && audit
                        .participants
                        .iter()
                        .any(|participant| participant.staged_writes != 0))
            {
                return invalid_snapshot("transition audit evidence is inconsistent");
            }
        }
        if pending
            .values()
            .any(|pending| pending.manifest.ready_at <= record.id)
        {
            return invalid_snapshot(
                "a transition manifest passed its ready boundary without an audit",
            );
        }
        for registered in late {
            insert_snapshot_pending(&mut pending, registered)?;
        }
    }
    if !pending
        .values()
        .eq(snapshot.pending_transition_manifests.iter())
    {
        return invalid_snapshot(
            "boundary transition evidence does not reconstruct the persisted pending manifests",
        );
    }
    Ok(())
}
