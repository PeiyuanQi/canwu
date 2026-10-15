//! Who appoints and removes auditors: what the appointment actions may do
//! under the [`FiscalAppointmentPolicy`] of the audited institution, and
//! what they change.
//!
//! Admission runs [`check_appointment`] against the committed state, and
//! settlement runs it again against the state the action settles on, so a
//! decision cites only what is still pending when it settles.

use crate::model::{
    FISCAL_APPOINTMENT_POLICY_MISSING, FISCAL_ISSUER_NOT_APPOINTER, FISCAL_ISSUER_NOT_LISTED,
    FISCAL_NOMINATION_REQUIRED, FISCAL_PETITION_REQUIRED, FISCAL_PROPOSAL_NOT_PENDING,
    FiscalAction, FiscalActionIssuer, FiscalActionRequest, FiscalAppointmentDecision,
    FiscalAppointmentParty, FiscalAppointmentPolicy, FiscalAuditBinding, FiscalAuditProposal,
    FiscalAuditProposalKind, FiscalIssuerRole, FiscalProposalOutcome, FiscalProposalRef,
    FiscalProposalResolution, FiscalRebinding, FiscalState, MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY,
    MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY, invalid,
};
use crate::plugin::{invalid_authority, invalid_payload, not_found, rebinding};
use canwu_api::{CanwuError, DomainRecordVersionRef, EntityRef, ErrorCode, SimTime};
use std::collections::{BTreeMap, BTreeSet};

/// Whether an action changes who holds a binding, or takes part in deciding
/// that: the rebinds, the appointment actions, and re-pointing or removing an
/// observer binding. Only the principal and institution roles issue them,
/// and a person a policy lists issues a nomination or petition; an acting
/// actor never does.
pub(crate) fn is_appointment_action(action: &FiscalAction) -> bool {
    matches!(
        action,
        FiscalAction::RebindAuthority { .. }
            | FiscalAction::RebindAudit { .. }
            | FiscalAction::NominateAuditor { .. }
            | FiscalAction::PetitionAuditorRemoval { .. }
            | FiscalAction::DecideAuditAppointment { .. }
            | FiscalAction::RebindObserver { .. }
            | FiscalAction::RemoveObserver { .. }
    )
}

/// Whether an action is a nomination or removal petition, the only actions a
/// person listed by an appointment policy issues.
pub(crate) fn is_proposal(action: &FiscalAction) -> bool {
    matches!(
        action,
        FiscalAction::NominateAuditor { .. } | FiscalAction::PetitionAuditorRemoval { .. }
    )
}

/// The authority basis an action would install or nominate, which must be
/// current when its command is admitted and again when it settles: a
/// rebind's, a nomination's, a direct appointment's, or the basis named by
/// the nomination an appointment accepts.
pub(crate) fn installed_basis<'a>(
    state: &'a FiscalState,
    action: &'a FiscalAction,
) -> Option<&'a DomainRecordVersionRef> {
    match action {
        FiscalAction::RebindAuthority {
            authority_basis, ..
        }
        | FiscalAction::RebindAudit {
            authority_basis, ..
        } => authority_basis.as_ref(),
        FiscalAction::NominateAuditor {
            authority_basis, ..
        }
        | FiscalAction::DecideAuditAppointment {
            decision:
                FiscalAppointmentDecision::Appoint {
                    authority_basis, ..
                },
            ..
        } => Some(authority_basis),
        FiscalAction::DecideAuditAppointment {
            decision: FiscalAppointmentDecision::AcceptNomination { nomination_id },
            ..
        } => match state.audit_proposals.get(nomination_id).map(|p| &p.kind) {
            Some(FiscalAuditProposalKind::Nomination {
                authority_basis, ..
            }) => Some(authority_basis),
            _ => None,
        },
        _ => None,
    }
}

/// Checks that `issuer` may take the part `request` takes in appointing or
/// removing an auditor, under the policy that governs the audited
/// institution, and that what the request cites is pending on its binding.
/// Any other action passes.
pub(crate) fn check_appointment(
    state: &FiscalState,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
) -> Result<(), CanwuError> {
    match &request.action {
        FiscalAction::NominateAuditor {
            audit_binding_id,
            institution,
            ..
        } => {
            require_binding_slot(state, audit_binding_id, institution)?;
            let policy = policy_for(state, institution)?;
            require_listed(state, policy, issuer, &policy.nominators).map(drop)
        }
        FiscalAction::PetitionAuditorRemoval {
            audit_binding_id, ..
        } => {
            let binding = audit_binding(state, audit_binding_id)?;
            let policy = policy_for(state, &binding.institution)?;
            require_listed(state, policy, issuer, &policy.petitioners).map(drop)
        }
        FiscalAction::DecideAuditAppointment {
            audit_binding_id,
            decision,
        } => {
            let institution = decided_institution(state, audit_binding_id, decision)?;
            let policy = policy_for(state, institution)?;
            if acting_institution(state, issuer) != Some(&policy.appointer) {
                return Err(invalid_authority(FISCAL_ISSUER_NOT_APPOINTER));
            }
            match decision {
                FiscalAppointmentDecision::AcceptNomination { nomination_id } => {
                    let (_, candidate, _) = nomination(state, nomination_id, audit_binding_id)?;
                    require_binding_slot(state, audit_binding_id, institution)?;
                    require_tenure(state, policy, audit_binding_id, candidate)
                }
                FiscalAppointmentDecision::Appoint { .. }
                    if !policy.appoints_without_nomination =>
                {
                    Err(invalid_authority(FISCAL_NOMINATION_REQUIRED))
                }
                FiscalAppointmentDecision::Appoint { auditor, .. } => {
                    require_binding_slot(state, audit_binding_id, institution)?;
                    require_tenure(state, policy, audit_binding_id, *auditor)
                }
                FiscalAppointmentDecision::AcceptPetition { petition_id } => {
                    petition(state, petition_id, audit_binding_id).map(drop)
                }
                FiscalAppointmentDecision::Remove if !policy.removes_without_petition => {
                    Err(invalid_authority(FISCAL_PETITION_REQUIRED))
                }
                FiscalAppointmentDecision::Remove | FiscalAppointmentDecision::Dismiss { .. } => {
                    Ok(())
                }
            }
        }
        FiscalAction::RebindAudit {
            audit_binding_id,
            auditor,
            ..
        } => {
            let institution = acting_institution(state, issuer)
                .ok_or_else(|| invalid_authority(FISCAL_ISSUER_NOT_APPOINTER))?;
            require_binding_slot(state, audit_binding_id, institution)?;
            let policy = policy_for(state, institution)?;
            if policy.appointer != *institution {
                return Err(invalid_authority(FISCAL_ISSUER_NOT_APPOINTER));
            }
            match auditor {
                Some(_) if !policy.appoints_without_nomination => {
                    Err(invalid_authority(FISCAL_NOMINATION_REQUIRED))
                }
                Some(auditor) => require_tenure(state, policy, audit_binding_id, *auditor),
                None if !policy.removes_without_petition => {
                    Err(invalid_authority(FISCAL_PETITION_REQUIRED))
                }
                None => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

/// Requires that appointing `auditor` to a binding displaces nobody the
/// policy protects. Replacing a seated auditor by another is a removal and
/// an appointment, so it needs a policy that lets the appointer remove
/// without a petition; appointing into a vacancy, or the seated auditor
/// again, does not. An auditor whose removal a petition asks for is removed
/// by accepting that petition first.
fn require_tenure(
    state: &FiscalState,
    policy: &FiscalAppointmentPolicy,
    audit_binding_id: &str,
    auditor: canwu_api::PersonId,
) -> Result<(), CanwuError> {
    match state.audit_bindings.get(audit_binding_id) {
        Some(seated) if seated.auditor != auditor && !policy.removes_without_petition => {
            Err(invalid_authority(FISCAL_PETITION_REQUIRED))
        }
        _ => Ok(()),
    }
}

/// Refuses, at admission, a nomination or petition its party has no room
/// for: one past [`MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY`] that does not
/// replace the party's pending proposal of the same kind on the same
/// binding. Settlement checks the same against the state the proposal
/// settles on.
pub(crate) fn admit_proposal_capacity(
    state: &FiscalState,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
) -> Result<(), CanwuError> {
    let (binding_id, institution, nominates) = match &request.action {
        FiscalAction::NominateAuditor {
            audit_binding_id,
            institution,
            ..
        } => (audit_binding_id, institution, true),
        FiscalAction::PetitionAuditorRemoval {
            audit_binding_id, ..
        } => (
            audit_binding_id,
            &audit_binding(state, audit_binding_id)?.institution,
            false,
        ),
        _ => return Ok(()),
    };
    let policy = policy_for(state, institution)?;
    let party = issuer_party(state, policy, issuer)
        .ok_or_else(|| invalid_authority(FISCAL_ISSUER_NOT_LISTED))?;
    replaced_proposal(state, institution, &party, binding_id, nominates).map(drop)
}

/// Applies a current, authorized appointment action to `state`: records a
/// nomination or petition, or carries out the appointer's decision. The
/// caller validates the result.
pub(crate) fn apply_appointment(
    state: &mut FiscalState,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
    at: SimTime,
) -> Result<(), CanwuError> {
    let decided = rebinding(request, issuer, at);
    let applied = match &request.action {
        FiscalAction::NominateAuditor {
            nomination_id,
            audit_binding_id,
            institution,
            candidate,
            authority_basis,
        } => open_proposal(
            state,
            issuer,
            decided,
            (nomination_id, audit_binding_id, institution.clone()),
            FiscalAuditProposalKind::Nomination {
                candidate: *candidate,
                authority_basis: authority_basis.clone(),
            },
        ),
        FiscalAction::PetitionAuditorRemoval {
            petition_id,
            audit_binding_id,
            grounds,
        } => {
            let binding = audit_binding(state, audit_binding_id)?;
            let (institution, auditor) = (binding.institution.clone(), binding.auditor);
            open_proposal(
                state,
                issuer,
                decided,
                (petition_id, audit_binding_id, institution),
                FiscalAuditProposalKind::RemovalPetition {
                    auditor,
                    grounds: grounds.clone(),
                },
            )
        }
        FiscalAction::DecideAuditAppointment {
            audit_binding_id,
            decision,
        } => decide(state, audit_binding_id, decision, &decided),
        FiscalAction::RebindAudit {
            audit_binding_id,
            auditor,
            authority_basis,
        } => {
            let institution = acting_institution(state, issuer)
                .cloned()
                .ok_or_else(|| invalid_authority(FISCAL_ISSUER_NOT_APPOINTER))?;
            match (auditor, authority_basis) {
                (Some(auditor), Some(authority_basis)) => {
                    install(
                        state,
                        FiscalAuditBinding {
                            id: audit_binding_id.clone(),
                            institution,
                            auditor: *auditor,
                            authority_basis: authority_basis.clone(),
                            rebound_by: Some(decided.clone()),
                            nomination: None,
                        },
                        &decided,
                    );
                    Ok(())
                }
                (None, None) => remove(state, audit_binding_id, &decided),
                _ => Err(invalid(
                    "a fiscal audit rebind names an auditor together with one exact authority basis",
                )),
            }
        }
        _ => Ok(()),
    };
    prune_resolved(state);
    applied
}

fn decide(
    state: &mut FiscalState,
    audit_binding_id: &str,
    decision: &FiscalAppointmentDecision,
    decided: &FiscalRebinding,
) -> Result<(), CanwuError> {
    match decision {
        FiscalAppointmentDecision::AcceptNomination { nomination_id } => {
            let (proposal, candidate, authority_basis) =
                nomination(state, nomination_id, audit_binding_id)?;
            let binding = FiscalAuditBinding {
                id: audit_binding_id.to_owned(),
                institution: proposal.institution.clone(),
                auditor: candidate,
                authority_basis: authority_basis.clone(),
                rebound_by: Some(decided.clone()),
                nomination: Some(FiscalProposalRef {
                    proposal_id: proposal.id.clone(),
                    proposed_by: proposal.proposed_by.issuer.clone(),
                }),
            };
            resolve(
                state,
                nomination_id,
                FiscalProposalOutcome::Accepted,
                decided,
            );
            install(state, binding, decided);
            Ok(())
        }
        FiscalAppointmentDecision::Appoint {
            institution,
            auditor,
            authority_basis,
        } => {
            install(
                state,
                FiscalAuditBinding {
                    id: audit_binding_id.to_owned(),
                    institution: institution.clone(),
                    auditor: *auditor,
                    authority_basis: authority_basis.clone(),
                    rebound_by: Some(decided.clone()),
                    nomination: None,
                },
                decided,
            );
            Ok(())
        }
        FiscalAppointmentDecision::AcceptPetition { petition_id } => {
            petition(state, petition_id, audit_binding_id)?;
            resolve(state, petition_id, FiscalProposalOutcome::Accepted, decided);
            remove(state, audit_binding_id, decided)
        }
        FiscalAppointmentDecision::Remove => remove(state, audit_binding_id, decided),
        FiscalAppointmentDecision::Dismiss { proposal_id } => {
            pending_proposal(state, proposal_id, audit_binding_id)?;
            resolve(
                state,
                proposal_id,
                FiscalProposalOutcome::Dismissed,
                decided,
            );
            Ok(())
        }
    }
}

/// Records a new proposal, `(id, audit binding, audited institution)` of
/// `kind`, made by `issuer` in the action `proposed`. It replaces the pending
/// proposal of the same kind that its party made on the same binding, which
/// is dropped rather than kept, and otherwise needs room among the party's
/// pending proposals.
fn open_proposal(
    state: &mut FiscalState,
    issuer: &FiscalActionIssuer,
    proposed: FiscalRebinding,
    (id, audit_binding_id, institution): (&String, &String, EntityRef),
    kind: FiscalAuditProposalKind,
) -> Result<(), CanwuError> {
    if state.audit_proposals.contains_key(id) {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "fiscal nomination or petition identity already exists",
        ));
    }
    let policy = policy_for(state, &institution)?;
    let party = issuer_party(state, policy, issuer)
        .ok_or_else(|| invalid_authority(FISCAL_ISSUER_NOT_LISTED))?;
    let nominates = matches!(kind, FiscalAuditProposalKind::Nomination { .. });
    if let Some(replaced) =
        replaced_proposal(state, &institution, &party, audit_binding_id, nominates)?
    {
        state.audit_proposals.remove(&replaced);
    }
    state.audit_proposals.insert(
        id.clone(),
        FiscalAuditProposal {
            id: id.clone(),
            audit_binding_id: audit_binding_id.clone(),
            institution,
            kind,
            party,
            proposed_by: proposed,
            resolution: None,
        },
    );
    Ok(())
}

/// The pending proposal of one kind that `party` has on a binding of
/// `institution`, which a new one replaces; or, when it has none, room for
/// the new one among the party's pending proposals of that institution.
fn replaced_proposal(
    state: &FiscalState,
    institution: &EntityRef,
    party: &FiscalAppointmentParty,
    audit_binding_id: &str,
    nominates: bool,
) -> Result<Option<String>, CanwuError> {
    let mut own = state.audit_proposals.values().filter(|proposal| {
        proposal.is_pending() && proposal.institution == *institution && proposal.party == *party
    });
    let mut count = 0_usize;
    let replaced = own.find_map(|proposal| {
        count += 1;
        (proposal.audit_binding_id == audit_binding_id
            && matches!(proposal.kind, FiscalAuditProposalKind::Nomination { .. }) == nominates)
            .then(|| proposal.id.clone())
    });
    if replaced.is_some() {
        return Ok(replaced);
    }
    if count >= MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "the party already has as many nominations and petitions pending as it may",
        ));
    }
    Ok(None)
}

/// Installs or replaces an audit binding. Every other nomination pending on
/// the binding is superseded, since the appointer has decided who holds it;
/// the petitions pending on it are superseded only when the seated auditor
/// they name is replaced. Proposals of other institutions are untouched.
fn install(state: &mut FiscalState, binding: FiscalAuditBinding, decided: &FiscalRebinding) {
    let replaced = state
        .audit_bindings
        .get(&binding.id)
        .is_some_and(|seated| seated.auditor != binding.auditor);
    supersede(
        state,
        &binding.id,
        &binding.institution,
        decided,
        |kind| match kind {
            FiscalAuditProposalKind::Nomination { .. } => true,
            FiscalAuditProposalKind::RemovalPetition { .. } => replaced,
        },
    );
    state.audit_bindings.insert(binding.id.clone(), binding);
}

/// Removes an audit binding, superseding every other petition pending on it.
/// Nominations stay pending for the vacancy.
fn remove(
    state: &mut FiscalState,
    audit_binding_id: &str,
    decided: &FiscalRebinding,
) -> Result<(), CanwuError> {
    let removed = state
        .audit_bindings
        .remove(audit_binding_id)
        .ok_or_else(|| not_found("fiscal audit binding is unavailable"))?;
    supersede(
        state,
        audit_binding_id,
        &removed.institution,
        decided,
        |kind| matches!(kind, FiscalAuditProposalKind::RemovalPetition { .. }),
    );
    Ok(())
}

/// Resolves as superseded the proposals pending on one binding of one
/// institution that `applies` selects.
fn supersede(
    state: &mut FiscalState,
    audit_binding_id: &str,
    institution: &EntityRef,
    decided: &FiscalRebinding,
    applies: impl Fn(&FiscalAuditProposalKind) -> bool,
) {
    for proposal in state.audit_proposals.values_mut().filter(|proposal| {
        proposal.is_pending()
            && proposal.audit_binding_id == audit_binding_id
            && proposal.institution == *institution
            && applies(&proposal.kind)
    }) {
        proposal.resolution = Some(FiscalProposalResolution {
            outcome: FiscalProposalOutcome::Superseded,
            resolved_by: decided.clone(),
        });
    }
}

/// Keeps at most [`MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY`] resolved
/// proposals of each policy, dropping the oldest: the earliest resolved,
/// then the lowest ID.
fn prune_resolved(state: &mut FiscalState) {
    let mut resolved = BTreeMap::<&EntityRef, Vec<(SimTime, &String)>>::new();
    for proposal in state.audit_proposals.values() {
        if let Some(resolution) = &proposal.resolution {
            resolved
                .entry(&proposal.institution)
                .or_default()
                .push((resolution.resolved_by.rebound_at, &proposal.id));
        }
    }
    let dropped: Vec<String> = resolved
        .into_values()
        .filter(|kept| kept.len() > MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY)
        .flat_map(|mut kept| {
            kept.sort();
            let excess = kept.len() - MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY;
            kept.into_iter()
                .take(excess)
                .map(|(_, id)| id.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    for id in dropped {
        state.audit_proposals.remove(&id);
    }
}

fn resolve(
    state: &mut FiscalState,
    proposal_id: &str,
    outcome: FiscalProposalOutcome,
    decided: &FiscalRebinding,
) {
    if let Some(proposal) = state.audit_proposals.get_mut(proposal_id) {
        proposal.resolution = Some(FiscalProposalResolution {
            outcome,
            resolved_by: decided.clone(),
        });
    }
}

/// The policy that governs the auditors of `institution`.
fn policy_for<'a>(
    state: &'a FiscalState,
    institution: &EntityRef,
) -> Result<&'a FiscalAppointmentPolicy, CanwuError> {
    state
        .audit_appointment_policies
        .values()
        .find(|policy| policy.institution == *institution)
        .ok_or_else(|| invalid_authority(FISCAL_APPOINTMENT_POLICY_MISSING))
}

/// The institution an issuer acts for under one of its authority bindings, as
/// that binding's principal or as a body; `None` for any other role.
fn acting_institution<'a>(
    state: &'a FiscalState,
    issuer: &FiscalActionIssuer,
) -> Option<&'a EntityRef> {
    matches!(
        issuer.role,
        FiscalIssuerRole::Principal | FiscalIssuerRole::Institution
    )
    .then(|| state.authority_bindings.get(&issuer.binding_id))
    .flatten()
    .map(|binding| &binding.institution)
}

/// The party `issuer` is under `policy`: the institution it acts for, or, for
/// a person acting under the policy itself, that person.
fn issuer_party(
    state: &FiscalState,
    policy: &FiscalAppointmentPolicy,
    issuer: &FiscalActionIssuer,
) -> Option<FiscalAppointmentParty> {
    if issuer.role == FiscalIssuerRole::ListedActor {
        return issuer
            .actor
            .filter(|_| issuer.binding_id == policy.id)
            .map(|actor| FiscalAppointmentParty::Actor { actor });
    }
    acting_institution(state, issuer).map(|institution| FiscalAppointmentParty::Institution {
        institution: institution.clone(),
    })
}

fn require_listed(
    state: &FiscalState,
    policy: &FiscalAppointmentPolicy,
    issuer: &FiscalActionIssuer,
    parties: &BTreeSet<FiscalAppointmentParty>,
) -> Result<FiscalAppointmentParty, CanwuError> {
    issuer_party(state, policy, issuer)
        .filter(|party| parties.contains(party))
        .ok_or_else(|| invalid_authority(FISCAL_ISSUER_NOT_LISTED))
}

/// Requires `audit_binding_id` to be usable for an audit binding on
/// `institution`: no authority binding or appointment policy has the ID, and
/// an audit binding that has it is on `institution`.
fn require_binding_slot(
    state: &FiscalState,
    audit_binding_id: &str,
    institution: &EntityRef,
) -> Result<(), CanwuError> {
    if state.authority_bindings.contains_key(audit_binding_id)
        || state
            .audit_appointment_policies
            .contains_key(audit_binding_id)
    {
        return Err(invalid_payload(
            "a fiscal audit binding takes an ID no authority binding uses and no appointment policy has",
        ));
    }
    if state
        .audit_bindings
        .get(audit_binding_id)
        .is_some_and(|binding| binding.institution != *institution)
    {
        return Err(invalid_authority(
            "a fiscal audit binding is rebound only from its audited institution",
        ));
    }
    Ok(())
}

pub(crate) fn audit_binding<'a>(
    state: &'a FiscalState,
    audit_binding_id: &str,
) -> Result<&'a FiscalAuditBinding, CanwuError> {
    state
        .audit_bindings
        .get(audit_binding_id)
        .ok_or_else(|| not_found("fiscal audit binding is unavailable"))
}

/// The institution whose auditors a decision concerns: the cited proposal's,
/// the one an appointment names, or the decided binding's.
pub(crate) fn decided_institution<'a>(
    state: &'a FiscalState,
    audit_binding_id: &str,
    decision: &'a FiscalAppointmentDecision,
) -> Result<&'a EntityRef, CanwuError> {
    match decision {
        FiscalAppointmentDecision::AcceptNomination { nomination_id: id }
        | FiscalAppointmentDecision::Dismiss { proposal_id: id } => {
            Ok(&pending_proposal(state, id, audit_binding_id)?.institution)
        }
        FiscalAppointmentDecision::Appoint { institution, .. } => Ok(institution),
        FiscalAppointmentDecision::AcceptPetition { .. } | FiscalAppointmentDecision::Remove => {
            Ok(&audit_binding(state, audit_binding_id)?.institution)
        }
    }
}

/// The proposal `proposal_id`, which must be pending on `audit_binding_id`.
fn pending_proposal<'a>(
    state: &'a FiscalState,
    proposal_id: &str,
    audit_binding_id: &str,
) -> Result<&'a FiscalAuditProposal, CanwuError> {
    let proposal = state
        .audit_proposals
        .get(proposal_id)
        .ok_or_else(|| not_found("fiscal nomination or petition is unavailable"))?;
    if !proposal.is_pending() || proposal.audit_binding_id != audit_binding_id {
        return Err(invalid_payload(FISCAL_PROPOSAL_NOT_PENDING));
    }
    Ok(proposal)
}

/// The pending nomination `nomination_id` on a binding, its candidate, and
/// the basis it names.
fn nomination<'a>(
    state: &'a FiscalState,
    nomination_id: &str,
    audit_binding_id: &str,
) -> Result<
    (
        &'a FiscalAuditProposal,
        canwu_api::PersonId,
        &'a DomainRecordVersionRef,
    ),
    CanwuError,
> {
    let proposal = pending_proposal(state, nomination_id, audit_binding_id)?;
    match &proposal.kind {
        FiscalAuditProposalKind::Nomination {
            candidate,
            authority_basis,
        } => Ok((proposal, *candidate, authority_basis)),
        FiscalAuditProposalKind::RemovalPetition { .. } => Err(invalid_payload(
            "an appointment accepts a nomination, not a petition",
        )),
    }
}

/// The pending petition `petition_id` on a binding, which must still name
/// the binding's auditor.
fn petition<'a>(
    state: &'a FiscalState,
    petition_id: &str,
    audit_binding_id: &str,
) -> Result<&'a FiscalAuditProposal, CanwuError> {
    let proposal = pending_proposal(state, petition_id, audit_binding_id)?;
    match &proposal.kind {
        FiscalAuditProposalKind::RemovalPetition { auditor, .. }
            if audit_binding(state, audit_binding_id)?.auditor == *auditor =>
        {
            Ok(proposal)
        }
        FiscalAuditProposalKind::RemovalPetition { .. } => {
            Err(invalid_payload(FISCAL_PROPOSAL_NOT_PENDING))
        }
        FiscalAuditProposalKind::Nomination { .. } => Err(invalid_payload(
            "a removal accepts a petition, not a nomination",
        )),
    }
}
