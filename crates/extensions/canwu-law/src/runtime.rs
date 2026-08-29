use crate::PLUGIN_NAMESPACE;
use crate::model::*;
use canwu_api::{
    Canwu, CanwuError, Command, CommandId, DECISION_REQUEST_COMMITMENT_DOMAIN, DecisionAction,
    DecisionAttemptOutcome, DecisionAttemptRecord, DecisionAuthority, DecisionContext,
    DecisionControllerBinding, DecisionIngressRequest, DecisionMutation, DecisionOption,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId, DecisionTicket,
    DecisionTicketDraft, DecisionTicketId, DomainRecordDraft, DomainRecordRef, EntityRef,
    EvidenceRef, IDENTITY_EVIDENCE_DEPENDENCIES_FIELD, IdentityEvidenceDependenciesV1,
    KnowledgeHolderRef, KnowledgeQuery, SimDuration, SimTime, canonical_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalSignal {
    pub kind: String,
    pub proposal_id: String,
    pub evidence: Vec<EvidenceRef>,
}

/// Canonical live mutations accepted by the law plugin at a Canwu boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LegalMutation {
    SubmitProposal {
        proposal: LegalProposal,
    },
    AdmitNonProceduralSource {
        proposal: LegalProposal,
    },
    Signal {
        signal: LegalSignal,
    },
    RetireCulturalTarget {
        target: CulturalTargetGenerationRef,
        reason: String,
    },
    RecordCase {
        case: LegalCase,
    },
    RecordFinding {
        finding: LegalFindingVersion,
    },
    RecordRuling {
        ruling: LegalRulingVersion,
    },
    RecordConflict {
        conflict: LegalConflict,
    },
    RecordPublicity {
        publicity: LegalPublicityEvent,
    },
    RecordSuccession {
        succession: LegalOrderSuccession,
    },
    AdmitCapacity {
        allocation: LegalCapacityAllocation,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalBoundaryResult {
    pub boundary: u64,
    pub adopted_proposals: Vec<String>,
    pub rejected_intents: Vec<LegalIntentOutcome>,
    pub applied_versions: Vec<String>,
    pub refreshed_effects: usize,
    pub emitted_outbox: Vec<LegalDecisionOutboxItem>,
}

/// Mutable, serializable legal ledger for one exact compiled plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalRuntime {
    pub plan: CompiledLawPlan,
    pub plan_hash: String,
    pub budgets: LawBudgets,
    pub reserved_state_bytes: usize,
    pub boundary_index: u64,
    pub last_settled_at: SimTime,
    pub next_outbox_sequence: u64,
    pub next_source_ordinal: u64,
    pub proposals: BTreeMap<String, LegalProposal>,
    pub procedures: BTreeMap<String, ProcedureInstance>,
    pub open_procedures: BTreeSet<String>,
    pub dirty_procedures: BTreeSet<String>,
    pub procedures_by_deadline: BTreeMap<SimTime, BTreeSet<String>>,
    pub capacity_allocations: BTreeMap<String, LegalCapacityAllocation>,
    pub participations: Vec<ProcedureParticipation>,
    pub latest_participation_by_key: BTreeMap<String, usize>,
    pub outbox: BTreeMap<u64, LegalDecisionOutboxItem>,
    pub outbox_keys: BTreeSet<String>,
    pub outbox_sequence_by_key: BTreeMap<String, u64>,
    pub pending_outbox_sequences: BTreeSet<u64>,
    pub staged_actor_contexts: BTreeMap<String, LegalActorContext>,
    pub pending_intents: BTreeMap<String, PendingLegalIntent>,
    pub consumed_intent_ids: BTreeSet<String>,
    pub intent_outcomes: Vec<LegalIntentOutcome>,
    pub sources: BTreeMap<String, LegalSourceVersion>,
    pub publicity_events: BTreeMap<String, LegalPublicityEvent>,
    pub rules: BTreeMap<String, LegalRule>,
    pub law_versions: BTreeMap<String, LawVersion>,
    pub law_versions_by_rule: BTreeMap<String, Vec<LegalRecordRef>>,
    pub rule_ids_by_order: BTreeMap<String, BTreeSet<String>>,
    pub scheduled_versions_by_time: BTreeMap<SimTime, Vec<LegalRecordRef>>,
    pub scheduled_live_dependencies:
        BTreeMap<CulturalTargetGenerationRef, BTreeSet<LegalRecordRef>>,
    pub scheduled_wakes: BTreeSet<SimTime>,
    pub dirty_rules: BTreeSet<String>,
    pub cases: BTreeMap<String, LegalCase>,
    pub findings: BTreeMap<String, LegalFindingVersion>,
    pub rulings: BTreeMap<String, LegalRulingVersion>,
    pub applicability: BTreeMap<String, ApplicabilityResult>,
    pub applicability_by_rule: BTreeMap<String, BTreeSet<String>>,
    pub conflicts: BTreeMap<String, LegalConflict>,
    pub conflict_ids_by_version: BTreeMap<String, BTreeSet<String>>,
    pub successions: Vec<LegalOrderSuccession>,
    pub succession_indexes_by_successor: BTreeMap<String, Vec<usize>>,
    pub retirements: Vec<LegalRetirement>,
    pub retired_cultural_targets: BTreeSet<CulturalTargetGenerationRef>,
    pub retained_evidence_dependencies: BTreeSet<EvidenceRef>,
    #[serde(with = "evidence_dependency_counts_serde")]
    pub retained_evidence_dependency_counts: BTreeMap<EvidenceRef, usize>,
}

impl LegalRuntime {
    #[must_use]
    pub fn new(plan: &CompiledLawPlan) -> Self {
        let mut state = Self {
            plan: plan.clone(),
            plan_hash: plan.content_hash.clone(),
            budgets: plan.budgets.clone(),
            reserved_state_bytes: 0,
            boundary_index: 0,
            last_settled_at: SimTime::EPOCH,
            next_outbox_sequence: 1,
            next_source_ordinal: 1,
            proposals: BTreeMap::new(),
            procedures: BTreeMap::new(),
            open_procedures: BTreeSet::new(),
            dirty_procedures: BTreeSet::new(),
            procedures_by_deadline: BTreeMap::new(),
            capacity_allocations: BTreeMap::new(),
            participations: Vec::new(),
            latest_participation_by_key: BTreeMap::new(),
            outbox: BTreeMap::new(),
            outbox_keys: BTreeSet::new(),
            outbox_sequence_by_key: BTreeMap::new(),
            pending_outbox_sequences: BTreeSet::new(),
            staged_actor_contexts: BTreeMap::new(),
            pending_intents: BTreeMap::new(),
            consumed_intent_ids: BTreeSet::new(),
            intent_outcomes: Vec::new(),
            sources: BTreeMap::new(),
            publicity_events: BTreeMap::new(),
            rules: BTreeMap::new(),
            law_versions: BTreeMap::new(),
            law_versions_by_rule: BTreeMap::new(),
            rule_ids_by_order: BTreeMap::new(),
            scheduled_versions_by_time: BTreeMap::new(),
            scheduled_live_dependencies: BTreeMap::new(),
            scheduled_wakes: BTreeSet::new(),
            dirty_rules: BTreeSet::new(),
            cases: BTreeMap::new(),
            findings: BTreeMap::new(),
            rulings: BTreeMap::new(),
            applicability: BTreeMap::new(),
            applicability_by_rule: BTreeMap::new(),
            conflicts: BTreeMap::new(),
            conflict_ids_by_version: BTreeMap::new(),
            successions: Vec::new(),
            succession_indexes_by_successor: BTreeMap::new(),
            retirements: Vec::new(),
            retired_cultural_targets: BTreeSet::new(),
            retained_evidence_dependencies: BTreeSet::new(),
            retained_evidence_dependency_counts: BTreeMap::new(),
        };
        let initial = state.persisted_payload_len().unwrap_or(usize::MAX);
        state.reserved_state_bytes = initial.saturating_add(256);
        state
    }

    pub fn from_state(plan: &CompiledLawPlan, state: Self) -> Result<Self, CanwuError> {
        state.validate_against_plan(plan)?;
        Ok(state)
    }

    fn ensure_plan_identity(&self, plan: &CompiledLawPlan) -> Result<(), CanwuError> {
        if self.plan != *plan || self.plan_hash != plan.content_hash || self.budgets != plan.budgets
        {
            return Err(invalid("legal runtime plan or budget identity mismatch"));
        }
        Ok(())
    }

    /// Checks the immutable plan binding used by the live plugin path.
    /// Full aggregate validation belongs to cold load, restore, and explicit
    /// diagnostics because it traverses all retained legal history.
    pub(crate) fn validate_live_plan_binding(
        &self,
        plan: &CompiledLawPlan,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        if self.proposals.len() > plan.budgets.max_procedures
            || self.procedures.len() > plan.budgets.max_procedures
            || self.sources.len() > plan.budgets.max_sources
            || self.publicity_events.len() > plan.budgets.max_sources
            || self.rules.len() > plan.budgets.max_rules
            || self.law_versions.len() > plan.budgets.max_law_versions
            || self.pending_intents.len() > plan.budgets.max_pending_intents
            || self.outbox.len() > plan.budgets.max_outbox
            || self.reserved_state_bytes > plan.budgets.max_state_bytes
            || self.reserved_state_bytes > plan.budgets.max_memory_bytes
        {
            return Err(invalid("legal runtime live budget exceeded"));
        }
        Ok(())
    }

    fn ensure_state_growth(&self, bytes: usize) -> Result<usize, CanwuError> {
        let reserved = self
            .reserved_state_bytes
            .checked_add(bytes)
            .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
        if reserved > self.budgets.max_state_bytes || reserved > self.budgets.max_memory_bytes {
            return Err(invalid("legal runtime serialized-state budget exhausted"));
        }
        Ok(reserved)
    }

    fn reserve_state_growth(&mut self, bytes: usize) -> Result<(), CanwuError> {
        self.reserved_state_bytes = self.ensure_state_growth(bytes)?;
        Ok(())
    }

    fn encoded_growth<T: Serialize + ?Sized>(
        value: &T,
        multiplier: usize,
    ) -> Result<usize, CanwuError> {
        serde_json::to_vec(value)
            .map_err(|error| invalid(format!("legal state growth cannot be encoded: {error}")))?
            .len()
            .checked_mul(multiplier)
            .and_then(|bytes| bytes.checked_add(256))
            .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))
    }

    /// Rebuilds the conservative byte account after an explicit cold import or fixture load.
    /// This serializes the full ledger and must not be called from ordinary settlement.
    pub fn reaccount_state_budget(&mut self) -> Result<(), CanwuError> {
        let encoded = self.persisted_payload_len()?;
        let reserved = encoded
            .checked_add(256)
            .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
        if reserved > self.budgets.max_state_bytes || reserved > self.budgets.max_memory_bytes {
            return Err(invalid("legal runtime serialized-state budget exhausted"));
        }
        self.reserved_state_bytes = reserved;
        Ok(())
    }

    pub fn validate_against_plan(&self, plan: &CompiledLawPlan) -> Result<(), CanwuError> {
        crate::validate_compiled_law_plan(plan)?;
        if self.plan != *plan || self.plan_hash != plan.content_hash || self.budgets != plan.budgets
        {
            return Err(invalid("legal runtime plan hash mismatch"));
        }
        // Reject an oversized cold snapshot before rebuilding any semantic
        // indexes. This keeps restore cost bounded by the serialized-state
        // budget instead of allowing an attacker to force full scans first.
        let encoded_bytes = self.persisted_payload_len()?;
        if encoded_bytes > plan.budgets.max_state_bytes
            || encoded_bytes > plan.budgets.max_memory_bytes
            || encoded_bytes > self.reserved_state_bytes
            || self.reserved_state_bytes > plan.budgets.max_state_bytes
            || self.reserved_state_bytes > plan.budgets.max_memory_bytes
        {
            return Err(invalid("legal runtime serialized-state budget exceeded"));
        }
        if self.proposals.len() > plan.budgets.max_procedures
            || self.procedures.len() > plan.budgets.max_procedures
            || self.capacity_allocations.len() > plan.budgets.max_procedures
            || self.sources.len() > plan.budgets.max_sources
            || self.publicity_events.len() > plan.budgets.max_sources
            || self.rules.len() > plan.budgets.max_rules
            || self.cases.len() > plan.budgets.max_cases
            || self.findings.len() > plan.budgets.max_findings
            || self.rulings.len() > plan.budgets.max_rulings
            || self.participations.len() > plan.budgets.max_participations
            || self.intent_outcomes.len() > plan.budgets.max_intent_outcomes
            || self.conflicts.len() > plan.budgets.max_conflicts
            || self.successions.len() > plan.budgets.max_successions
            || self.retirements.len() > plan.budgets.max_retirements
            || self.outbox.len() > plan.budgets.max_outbox
            || self.pending_intents.len() > plan.budgets.max_pending_intents
            || self.law_versions.len() > plan.budgets.max_law_versions
            || self.scheduled_wakes.len()
                > plan
                    .budgets
                    .max_procedures
                    .checked_add(plan.budgets.max_law_versions)
                    .ok_or_else(|| invalid("legal wake budget overflowed"))?
            || self.consumed_intent_ids.len()
                > plan
                    .budgets
                    .max_participations
                    .checked_add(plan.budgets.max_intent_outcomes)
                    .ok_or_else(|| invalid("legal consumed-intent budget overflowed"))?
        {
            return Err(invalid("legal runtime budget exceeded"));
        }
        let rebuilt_dependency_counts = self.rebuild_identity_evidence_dependency_counts();
        if self.retained_evidence_dependency_counts != rebuilt_dependency_counts
            || self.retained_evidence_dependencies
                != rebuilt_dependency_counts.keys().cloned().collect()
        {
            return Err(invalid(
                "legal runtime identity-evidence index is inconsistent",
            ));
        }
        for (id, proposal) in &self.proposals {
            validate_proposal_fanout(plan, proposal)?;
            let mut canonical_proposal = proposal.clone();
            canonicalize_proposal(&mut canonical_proposal);
            let source_profile = plan
                .source_profile_by_id
                .get(&proposal.source_profile)
                .and_then(|key| plan.source_profiles.get(key.get() as usize))
                .ok_or_else(|| invalid("legal proposal source profile is missing"))?;
            let procedure_matches = source_profile.procedure.as_ref().map_or_else(
                || proposal.procedure_profile.is_empty() && proposal.active_procedure.is_none(),
                |procedure| procedure == &proposal.procedure_profile,
            );
            self.validate_proposal_legal_claim(plan, source_profile, proposal)?;
            let publicity_event = proposal.publicity.as_ref().and_then(|reference| {
                self.publicity_events
                    .get(&reference.id)
                    .filter(|event| reference == &local_ref("publicity", &event.id))
            });
            let publicity_timeline_valid = match source_profile.publicity_policy {
                PublicityPolicy::ValidityCondition => match proposal.status {
                    ProposalStatus::Adopted => proposal.adopted_at.is_some_and(|adopted| {
                        publicity_event.is_some_and(|event| event.at <= adopted)
                    }),
                    _ => proposal.publicity.is_none() || publicity_event.is_some(),
                },
                PublicityPolicy::EffectivenessCondition => publicity_event.is_none_or(|event| {
                    event.at <= proposal.effective_at
                        && match proposal.status {
                            ProposalStatus::Adopted => {
                                let Some(adopted) = proposal.adopted_at else {
                                    return false;
                                };
                                proposal.source_version.as_ref().is_some_and(|reference| {
                                    self.sources.get(&reference.id).is_some_and(|source| {
                                        source.publicity_event.as_ref().map_or_else(
                                            || {
                                                adopted <= event.at
                                                    && source.promulgated_at.is_none()
                                                    && proposal.law_version.as_ref().is_some_and(
                                                        |version_ref| {
                                                            self.law_versions
                                                                .get(&version_ref.id)
                                                                .is_some_and(|version| {
                                                                    version.promulgated_at.is_none()
                                                                })
                                                        },
                                                    )
                                            },
                                            |source_event| {
                                                proposal.publicity.as_ref().is_some_and(
                                                    |reference| source_event == reference,
                                                ) && event.at <= adopted
                                                    && source.promulgated_at == Some(event.at)
                                            },
                                        )
                                    })
                                })
                            }
                            _ => true,
                        }
                }),
                PublicityPolicy::NotRequired => proposal.publicity.is_none(),
                PublicityPolicy::EvidenceOnly => {
                    proposal.publicity.is_none() || publicity_event.is_some()
                }
            };
            let lifecycle_links_valid = match proposal.status {
                ProposalStatus::Adopted => {
                    proposal.adopted_at.is_some()
                        && proposal.source_version.as_ref().is_some_and(|reference| {
                            self.sources.get(&reference.id).is_some_and(|source| {
                                reference == &local_ref("source_version", &source.id)
                                    && source.proposal == local_ref("proposal", &proposal.id)
                            })
                        })
                        && proposal
                            .source_version
                            .as_ref()
                            .is_some_and(|source_reference| {
                                proposal.law_version.as_ref().is_some_and(|reference| {
                                    self.law_versions.get(&reference.id).is_some_and(|version| {
                                        reference == &law_version_reference(version)
                                            && version.source == *source_reference
                                    })
                                })
                            })
                }
                ProposalStatus::Draft
                | ProposalStatus::Submitted
                | ProposalStatus::Deliberating
                | ProposalStatus::Rejected
                | ProposalStatus::Expired
                | ProposalStatus::Withdrawn => {
                    proposal.adopted_at.is_none()
                        && proposal.source_version.is_none()
                        && proposal.law_version.is_none()
                }
            };
            if id != &proposal.id
                || proposal.procedure_profile_hash != plan.content_hash
                || proposal.evidence.len() > plan.budgets.max_evidence_per_record
                || proposal
                    .cultural_dependencies
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || proposal
                    .expected_rule_head
                    .as_ref()
                    .is_some_and(|reference| {
                        self.law_versions
                            .get(&reference.id)
                            .is_none_or(|version| reference != &law_version_reference(version))
                    })
                || !procedure_matches
                || !publicity_timeline_valid
                || !lifecycle_links_valid
                || canonical_proposal.defects != proposal.defects
                || canonical_proposal.jurisdictions != proposal.jurisdictions
                || canonical_proposal.subjects != proposal.subjects
                || canonical_proposal.cultural_dependencies != proposal.cultural_dependencies
                || canonical_proposal.clauses != proposal.clauses
                || proposal.publicity.as_ref().is_some_and(|reference| {
                    self.publicity_events
                        .get(&reference.id)
                        .is_none_or(|event| {
                            reference != &local_ref("publicity", &event.id)
                                || event.proposal != local_ref("proposal", &proposal.id)
                        })
                })
            {
                return Err(invalid(
                    "legal proposal identity, profile, or evidence is invalid",
                ));
            }
            if !plan.order_by_id.contains_key(&proposal.legal_order)
                || !plan
                    .source_profile_by_id
                    .contains_key(&proposal.source_profile)
            {
                return Err(invalid(format!(
                    "proposal {} references unknown plan item",
                    proposal.id
                )));
            }
            for clause in &proposal.clauses {
                if !plan
                    .clauses
                    .iter()
                    .any(|candidate| candidate.id == clause.clause)
                {
                    return Err(invalid(format!(
                        "proposal {} references unknown clause",
                        proposal.id
                    )));
                }
            }
        }
        for (id, procedure) in &self.procedures {
            let profile = plan
                .procedure_by_id
                .get(&procedure.profile)
                .and_then(|key| plan.procedures.get(key.get() as usize))
                .ok_or_else(|| invalid("legal procedure has no compiled profile"))?;
            let expected_seats = profile
                .stages
                .get(procedure.active_stage)
                .map_or_else(Vec::new, |stage| stage.seats.clone());
            let expected_authority_seats = profile
                .stages
                .iter()
                .flat_map(|stage| &stage.seats)
                .cloned()
                .collect::<BTreeSet<_>>();
            let expected_authorities = expected_authority_seats
                .iter()
                .map(|seat| {
                    let (holder, permission_profile_id, decision_controller_id) =
                        seat_authority(plan, &procedure.profile, seat);
                    (
                        seat.clone(),
                        ProcedureSeatAuthority {
                            holder,
                            permission_profile_id,
                            decision_controller_id,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();
            if id != &procedure.id
                || procedure.profile_hash != plan.content_hash
                || procedure.stages != profile.stages
                || procedure.evidence.len() > plan.budgets.max_evidence_per_record
                || procedure.proposal != local_ref("proposal", &procedure.proposal.id)
                || self
                    .proposals
                    .get(&procedure.proposal.id)
                    .is_none_or(|proposal| {
                        proposal.active_procedure.as_deref() != Some(id.as_str())
                            || proposal.procedure_profile != procedure.profile
                    })
                || procedure.eligible_seats != expected_seats
                || procedure.seat_authorities != expected_authorities
            {
                return Err(invalid(format!(
                    "procedure {} has invalid identity, profile, evidence, or proposal",
                    procedure.id
                )));
            }
        }
        for (procedure_id, allocation) in &self.capacity_allocations {
            let procedure = self
                .procedures
                .get(procedure_id)
                .ok_or_else(|| invalid("legal capacity allocation has no procedure"))?;
            let profile = plan
                .procedure_by_id
                .get(&procedure.profile)
                .and_then(|key| plan.procedures.get(key.get() as usize))
                .ok_or_else(|| invalid("legal capacity allocation has no profile"))?;
            if allocation.procedure != *procedure_id
                || profile.reservation_pool.as_ref() != Some(&allocation.pool)
                || profile.reservation_quantity != allocation.quantity
            {
                return Err(invalid(
                    "legal capacity allocation does not match its compiled requirement",
                ));
            }
        }
        for intent in self.pending_intents.values() {
            if !self.procedures.contains_key(&intent.procedure.id) {
                return Err(invalid(format!("intent {} has no procedure", intent.id)));
            }
        }
        let expected_source_ordinal = self
            .sources
            .values()
            .map(|source| source.ordinal)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("legal source ordinal overflowed"))?;
        if self.next_source_ordinal != expected_source_ordinal {
            return Err(invalid("legal source ordinal counter is inconsistent"));
        }
        for (id, event) in &self.publicity_events {
            let profile = self
                .proposals
                .get(&event.proposal.id)
                .and_then(|proposal| plan.source_profile_by_id.get(&proposal.source_profile))
                .and_then(|key| plan.source_profiles.get(key.get() as usize));
            if id != &event.id
                || event.proposal.kind != "proposal"
                || event.at > self.last_settled_at
                || event.signal_kind.is_empty()
                || self
                    .proposals
                    .get(&event.proposal.id)
                    .is_none_or(|proposal| {
                        event.proposal != local_ref("proposal", &proposal.id)
                            || proposal.publicity.as_ref() != Some(&local_ref("publicity", id))
                    })
                || event.medium.is_empty()
                || event.scope.is_empty()
                || event.scope.len() > plan.budgets.max_jurisdictions_per_proposal
                || event.evidence.is_empty()
                || event.evidence.len() > plan.budgets.max_evidence_per_record
                || self
                    .proposals
                    .get(&event.proposal.id)
                    .is_none_or(|proposal| event.scope != proposal.jurisdictions)
                || profile.is_none_or(|profile| {
                    profile.publicity_policy == PublicityPolicy::NotRequired
                        || profile.publicity_signal_kind.as_ref() != Some(&event.signal_kind)
                })
            {
                return Err(invalid("legal publicity event is invalid"));
            }
        }
        for (id, source) in &self.sources {
            let source_origin_matches = match (source.mode, source.origin.as_ref()) {
                (SourceMode::Promulgated | SourceMode::Accreted, None)
                | (SourceMode::Agreed, Some(LegalOriginRef::Agreement { .. }))
                | (SourceMode::Received, Some(LegalOriginRef::Reception { .. })) => true,
                (SourceMode::Adjudicated, Some(LegalOriginRef::Ruling { ruling })) => {
                    self.rulings.get(&ruling.id).is_some_and(|record| {
                        ruling == &local_ref("ruling", &record.id)
                            && record.disposition == OperativeDisposition::Operative
                            && record.effective_from <= source.effective_at
                            && record
                                .effective_until
                                .is_none_or(|until| source.effective_at <= until)
                            && self
                                .proposals
                                .get(&source.proposal.id)
                                .is_some_and(|proposal| {
                                    proposal.jurisdictions.iter().all(|jurisdiction| {
                                        scope_covers(&record.scope, jurisdiction)
                                    })
                                })
                    })
                }
                _ => false,
            };
            if id != &source.id
                || source.evidence.len() > plan.budgets.max_evidence_per_record
                || self
                    .proposals
                    .get(&source.proposal.id)
                    .is_none_or(|proposal| {
                        source.proposal != local_ref("proposal", &proposal.id)
                            || source.legal_order != proposal.legal_order
                            || source.competence != proposal.competence
                            || source.validity != proposal.validity
                            || source.defects != proposal.defects
                            || source.origin != proposal.origin
                            || plan
                                .source_profile_by_id
                                .get(&proposal.source_profile)
                                .and_then(|key| plan.source_profiles.get(key.get() as usize))
                                .is_none_or(|profile| {
                                    profile.mode != source.mode
                                        || profile.authority_policy != source.authority_policy
                                        || profile.publicity_policy != source.publicity_policy
                                        || (source.publicity_event != proposal.publicity
                                            && !(profile.publicity_policy
                                                == PublicityPolicy::EffectivenessCondition
                                                && source.publicity_event.is_none()
                                                && proposal.publicity.is_some()))
                                        || (source.effective_at < source.adopted_at
                                            && (!profile.allow_retroactive
                                                || proposal.retrospective_from.is_none()))
                                })
                    })
                || source
                    .evidence_kinds
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || !plan.order_by_id.contains_key(&source.legal_order)
                || !plan.applicability_profiles.iter().any(|profile| {
                    profile.id == source.applicability_profile
                        && profile.legal_order == source.legal_order
                })
                || !legal_claim_fields_match(source.competence, &source.defects, source.validity)
                || !source_origin_matches
                || source.publicity_event.as_ref().is_some_and(|reference| {
                    self.publicity_events
                        .get(&reference.id)
                        .is_none_or(|event| {
                            reference != &local_ref("publicity", &event.id)
                                || event.proposal != source.proposal
                                || source.promulgated_at != Some(event.at)
                                || source.publicity != event.medium
                        })
                })
                || match source.publicity_policy {
                    PublicityPolicy::ValidityCondition => source.publicity_event.is_none(),
                    PublicityPolicy::EffectivenessCondition => {
                        source.publicity_event.is_none()
                            && !self
                                .proposals
                                .get(&source.proposal.id)
                                .is_some_and(|proposal| {
                                    proposal.status == ProposalStatus::Adopted
                                        && proposal.adopted_at.is_some()
                                        && proposal.law_version.as_ref().is_some_and(|reference| {
                                            self.law_versions.get(&reference.id).is_some_and(
                                                |version| version.source.id == source.id,
                                            )
                                        })
                                })
                    }
                    PublicityPolicy::NotRequired => source.publicity_event.is_some(),
                    PublicityPolicy::EvidenceOnly => false,
                }
                || source
                    .cultural_dependencies
                    .iter()
                    .any(|dependency| !source.evidence.contains(&dependency.evidence))
                || source.procedure.as_ref().is_some_and(|reference| {
                    reference.kind != "procedure"
                        || !self.procedures.contains_key(&reference.id)
                        || self
                            .procedures
                            .get(&reference.id)
                            .is_none_or(|procedure| reference != &procedure_reference(procedure))
                })
            {
                return Err(invalid("legal source identity or provenance is invalid"));
            }
        }
        let mut versions_by_rule_ordinal = BTreeMap::<(&str, u64), &LawVersion>::new();
        for (id, version) in &self.law_versions {
            if id != &version.id
                || version.evidence.len() > plan.budgets.max_evidence_per_record
                || !self.rules.contains_key(&version.rule)
                || self.sources.get(&version.source.id).is_none_or(|source| {
                    version.source != local_ref("source_version", &source.id)
                        || version.applicability_profile != source.applicability_profile
                        || version.adopted_at != source.adopted_at
                        || version.effective_at != source.effective_at
                        || version.promulgated_at != source.promulgated_at
                        || version.origin != source.origin
                        || self
                            .proposals
                            .get(&source.proposal.id)
                            .is_none_or(|proposal| {
                                version.retrospective_from != proposal.retrospective_from
                            })
                        || version.disposition
                            != if source.validity == OperativeDisposition::Operative {
                                disposition_for_operation(version.operation)
                            } else {
                                source.validity
                            }
                })
                || versions_by_rule_ordinal
                    .insert((version.rule.as_str(), version.legal_ordinal), version)
                    .is_some()
                || version
                    .deltas
                    .iter()
                    .flat_map(|effect| &effect.source_refs)
                    .any(|reference| reference != &version.source)
                || version
                    .cultural_dependencies
                    .iter()
                    .any(|dependency| !version.evidence.contains(&dependency.evidence))
            {
                return Err(invalid(format!(
                    "law version {} has invalid identity, source, or effects",
                    version.id
                )));
            }
        }
        for version in self.law_versions.values() {
            let mut expected_predecessors = if version.legal_ordinal == 1 {
                Vec::new()
            } else {
                let predecessor = versions_by_rule_ordinal
                    .get(&(version.rule.as_str(), version.legal_ordinal - 1))
                    .ok_or_else(|| invalid("legal version ordinal chain is not contiguous"))?;
                if version.effective_at < predecessor.effective_at {
                    return Err(invalid(
                        "legal version effective times must be monotonic within a rule",
                    ));
                }
                vec![law_version_reference(predecessor)]
            };
            if let Some(LegalOriginRef::Reception { predecessor, .. }) = &version.origin {
                expected_predecessors.push(predecessor.clone());
                expected_predecessors.sort();
                expected_predecessors.dedup();
            }
            if version.predecessors != expected_predecessors {
                return Err(invalid("legal version predecessor chain is not exact"));
            }
        }
        for (id, rule) in &self.rules {
            if id != &rule.id || !plan.order_by_id.contains_key(&rule.legal_order) {
                return Err(invalid("legal rule identity or order is invalid"));
            }
            let validate_reference = |reference: &LegalRecordRef| {
                self.law_versions.get(&reference.id).is_some_and(|version| {
                    version.rule == rule.id && reference == &law_version_reference(version)
                })
            };
            let versions = versions_by_rule_ordinal
                .range((rule.id.as_str(), 0)..=(rule.id.as_str(), u64::MAX))
                .map(|(_, version)| *version)
                .collect::<Vec<_>>();
            let latest = versions
                .last()
                .ok_or_else(|| invalid("legal rule latest version is missing"))?;
            let expected_latest = law_version_reference(latest);
            let expected_operative = versions
                .iter()
                .filter(|version| {
                    materializes_legal_effect(version)
                        && self.version_publicity_satisfied_at(plan, version, self.last_settled_at)
                        && version.effective_at <= self.last_settled_at
                })
                .max_by_key(|version| {
                    (
                        version.effective_at,
                        version.legal_ordinal,
                        version.id.as_str(),
                    )
                })
                .map(|version| law_version_reference(version));
            let mut expected_scheduled = versions
                .iter()
                .filter(|version| {
                    materializes_legal_effect(version)
                        && self.version_publicity_satisfied_at(plan, version, self.last_settled_at)
                        && version.effective_at > self.last_settled_at
                })
                .map(|version| law_version_reference(version))
                .collect::<Vec<_>>();
            expected_scheduled.sort();
            let expected_effects = expected_operative
                .as_ref()
                .and_then(|reference| self.law_versions.get(&reference.id))
                .map_or_else(Vec::new, |version| version.deltas.clone());
            let expected_retired = expected_operative
                .as_ref()
                .and_then(|reference| self.law_versions.get(&reference.id))
                .is_some_and(|version| retires_rule(version.operation));
            if rule.latest_adopted_version.as_ref() != Some(&expected_latest)
                || rule.operative_version != expected_operative
                || rule.scheduled_versions != expected_scheduled
                || rule.effects != expected_effects
                || rule.retired != expected_retired
                || rule
                    .latest_adopted_version
                    .as_ref()
                    .is_some_and(|reference| !validate_reference(reference))
            {
                return Err(invalid("legal rule derived state is inconsistent"));
            }
        }
        let mut expected_versions_by_rule = BTreeMap::<String, Vec<LegalRecordRef>>::new();
        for version in self.law_versions.values() {
            expected_versions_by_rule
                .entry(version.rule.clone())
                .or_default()
                .push(law_version_reference(version));
        }
        for references in expected_versions_by_rule.values_mut() {
            references.sort_by(|left, right| {
                let left = &self.law_versions[&left.id];
                let right = &self.law_versions[&right.id];
                (left.effective_at, left.legal_ordinal, &left.id).cmp(&(
                    right.effective_at,
                    right.legal_ordinal,
                    &right.id,
                ))
            });
        }
        if self.law_versions_by_rule != expected_versions_by_rule {
            return Err(invalid("legal rule-version query index is inconsistent"));
        }
        let mut expected_rules_by_order = BTreeMap::<String, BTreeSet<String>>::new();
        for rule in self.rules.values() {
            expected_rules_by_order
                .entry(rule.legal_order.clone())
                .or_default()
                .insert(rule.id.clone());
        }
        if self.rule_ids_by_order != expected_rules_by_order {
            return Err(invalid("legal order-rule query index is inconsistent"));
        }
        let expected_open = self
            .procedures
            .values()
            .filter(|procedure| !procedure.closed)
            .map(|procedure| procedure.id.clone())
            .collect::<BTreeSet<_>>();
        if self.open_procedures != expected_open {
            return Err(invalid("legal open-procedure index is inconsistent"));
        }
        if !self.dirty_procedures.is_subset(&self.open_procedures) {
            return Err(invalid("legal dirty-procedure index is inconsistent"));
        }
        let mut expected_deadlines = BTreeMap::<SimTime, BTreeSet<String>>::new();
        for procedure in self
            .procedures
            .values()
            .filter(|procedure| !procedure.closed)
        {
            expected_deadlines
                .entry(procedure_expiry_time(procedure.deadline)?)
                .or_default()
                .insert(procedure.id.clone());
        }
        if self.procedures_by_deadline != expected_deadlines {
            return Err(invalid("legal procedure deadline index is inconsistent"));
        }
        let mut participation_ids = BTreeSet::new();
        let mut expected_latest_participation = BTreeMap::<String, usize>::new();
        for (index, participation) in self.participations.iter().enumerate() {
            let procedure = self
                .procedures
                .get(&participation.procedure.id)
                .filter(|procedure| participation.procedure == procedure_reference(procedure))
                .ok_or_else(|| invalid("legal participation procedure is not exact"))?;
            let stage = procedure
                .stages
                .iter()
                .find(|stage| stage.id == participation.stage)
                .filter(|stage| stage.seats.contains(&participation.seat))
                .ok_or_else(|| invalid("legal participation stage or seat is invalid"))?;
            if !participation_ids.insert(participation.id.as_str()) {
                return Err(invalid("duplicate legal participation identity"));
            }
            let key = participation_key(
                &procedure.id,
                &participation.stage,
                participation.round,
                &participation.seat,
            );
            let expected_replaced = expected_latest_participation
                .get(&key)
                .map(|previous| local_ref("participation", &self.participations[*previous].id));
            if participation.replaced != expected_replaced
                || (expected_replaced.is_some() && !stage.allow_replacement)
            {
                return Err(invalid("legal participation replacement chain is invalid"));
            }
            expected_latest_participation.insert(key, index);
        }
        if self.latest_participation_by_key != expected_latest_participation {
            return Err(invalid("legal participation index is inconsistent"));
        }
        let expected_outbox = self
            .outbox
            .values()
            .map(|item| outbox_key(&item.procedure.id, item.stage, item.round, &item.seat))
            .collect::<BTreeSet<_>>();
        if self.outbox_keys != expected_outbox {
            return Err(invalid("legal outbox index is inconsistent"));
        }
        let expected_outbox_sequences = self
            .outbox
            .iter()
            .map(|(sequence, item)| {
                (
                    outbox_key(&item.procedure.id, item.stage, item.round, &item.seat),
                    *sequence,
                )
            })
            .collect::<BTreeMap<_, _>>();
        if self.outbox_sequence_by_key != expected_outbox_sequences {
            return Err(invalid("legal outbox sequence index is inconsistent"));
        }
        let expected_pending_outbox = self
            .outbox
            .iter()
            .filter(|(_, item)| item.dispatch == DispatchState::Pending)
            .map(|(sequence, _)| *sequence)
            .collect::<BTreeSet<_>>();
        if self.pending_outbox_sequences != expected_pending_outbox {
            return Err(invalid("legal pending-outbox index is inconsistent"));
        }
        let expected_outbox_sequence = self
            .outbox
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("legal outbox sequence overflowed"))?;
        if self.next_outbox_sequence != expected_outbox_sequence {
            return Err(invalid("legal outbox sequence counter is inconsistent"));
        }
        for (key, context) in &self.staged_actor_contexts {
            let valid = self.open_procedures.iter().any(|procedure_id| {
                self.procedures.get(procedure_id).is_some_and(|procedure| {
                    procedure
                        .stages
                        .get(procedure.active_stage)
                        .is_some_and(|stage| {
                            stage.seats.iter().any(|seat| {
                                key == &outbox_key(
                                    &procedure.id,
                                    procedure.active_stage,
                                    procedure.round,
                                    seat,
                                ) && procedure
                                    .seat_authorities
                                    .get(seat)
                                    .is_some_and(|authority| authority.holder == context.holder)
                            })
                        })
                })
            });
            if !valid
                || context.context_hash != actor_context_hash(context)?
                || context.knowledge_record_ids.len() > plan.budgets.max_evidence_per_record
            {
                return Err(invalid("staged legal actor context is invalid"));
            }
        }
        for (sequence, item) in &self.outbox {
            let ids = allocate_outbox_ids(plan, *sequence)?;
            let procedure = self
                .procedures
                .get(&item.procedure.id)
                .ok_or_else(|| invalid("legal outbox procedure is missing"))?;
            let proposal = self
                .proposals
                .get(&item.proposal.id)
                .ok_or_else(|| invalid("legal outbox proposal is missing"))?;
            let mut procedure_snapshot = procedure.clone();
            procedure_snapshot.active_stage = item.stage;
            procedure_snapshot.round = item.round;
            procedure_snapshot.deadline = item.expires_at;
            let stage = procedure_snapshot
                .stages
                .get(item.stage)
                .ok_or_else(|| invalid("legal outbox stage is missing"))?;
            let (controller, permission, decision_controller) =
                seat_authority(plan, &procedure.profile, &item.seat);
            let context = LegalActorContext {
                holder: item.controller.clone(),
                read_cut: item.knowledge_read_cut.clone(),
                knowledge_record_ids: item.knowledge_record_ids.clone(),
                facts: item
                    .draft
                    .context
                    .payload
                    .get("facts")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                context_hash: item.context_hash.clone(),
            };
            let expected_draft = decision_ticket_draft(
                proposal,
                &procedure_snapshot,
                stage,
                &item.seat,
                &controller,
                &decision_controller,
                ids.ticket,
                ids.command,
                item.due_at,
                &context,
            )?;
            if item.sequence != *sequence
                || item.ticket_id != ids.ticket
                || item.create_request_id != ids.create
                || item.refresh_request_id != Some(ids.refresh)
                || item.resolution_request_id != ids.resolution
                || item.nested_command_request_id != ids.command
                || item.proposal != local_ref("proposal", &proposal.id)
                || item.procedure != procedure_reference(procedure)
                || item.controller != controller
                || item.permission_profile_id != permission
                || item.decision_controller_id != decision_controller
                || item.command_subject != proposal.sponsor
                || item.draft != expected_draft
                || item.context_hash != actor_context_hash(&context)?
                || item.knowledge_record_ids.len() > plan.budgets.max_evidence_per_record
                || (item.dispatch == DispatchState::Acknowledged) != item.acknowledgement.is_some()
                || (matches!(
                    item.dispatch,
                    DispatchState::Enqueued | DispatchState::Acknowledged
                ) && item.enqueue_expected_revision.is_none())
                || (item.dispatch == DispatchState::Pending
                    && item.enqueue_outcome_commitment.is_some())
                || (matches!(
                    item.dispatch,
                    DispatchState::Enqueued | DispatchState::Acknowledged
                ) && item.enqueue_outcome_commitment.is_none())
                || item.enqueue_ingress.is_some() != item.enqueue_outcome_commitment.is_some()
                || (item.dispatch == DispatchState::Pending && item.enqueue_ingress.is_some())
                || (matches!(
                    item.dispatch,
                    DispatchState::Enqueued | DispatchState::Acknowledged
                ) && item.enqueue_ingress.is_none())
            {
                return Err(invalid("legal outbox identity or lifecycle is invalid"));
            }
        }
        if self
            .pending_intents
            .keys()
            .any(|id| self.consumed_intent_ids.contains(id))
        {
            return Err(invalid("consumed legal intent remains pending"));
        }
        let expected_consumed_intents = self
            .participations
            .iter()
            .filter_map(|participation| participation.id.strip_prefix("participation:"))
            .chain(
                self.intent_outcomes
                    .iter()
                    .map(|outcome| outcome.intent.as_str()),
            )
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        if self.consumed_intent_ids != expected_consumed_intents {
            return Err(invalid("consumed legal intent index is inconsistent"));
        }
        let mut expected_schedule = BTreeMap::<SimTime, Vec<LegalRecordRef>>::new();
        for rule in self.rules.values() {
            for reference in &rule.scheduled_versions {
                let version = self
                    .law_versions
                    .get(&reference.id)
                    .ok_or_else(|| invalid("scheduled law version is missing"))?;
                expected_schedule
                    .entry(version.effective_at)
                    .or_default()
                    .push(reference.clone());
            }
        }
        for references in expected_schedule.values_mut() {
            references.sort();
            references.dedup();
        }
        if self.scheduled_versions_by_time != expected_schedule {
            return Err(invalid("legal effective-time index is inconsistent"));
        }
        if self
            .scheduled_wakes
            .iter()
            .any(|scheduled| *scheduled < self.last_settled_at)
        {
            return Err(invalid("legal scheduled wake is stale"));
        }
        let mut expected_live_dependencies =
            BTreeMap::<CulturalTargetGenerationRef, BTreeSet<LegalRecordRef>>::new();
        for rule in self.rules.values() {
            for reference in &rule.scheduled_versions {
                let version = &self.law_versions[&reference.id];
                for dependency in &version.cultural_dependencies {
                    if dependency.kind == CulturalDependencyKind::LiveLevel {
                        expected_live_dependencies
                            .entry(dependency.target.clone())
                            .or_default()
                            .insert(reference.clone());
                    }
                }
            }
        }
        if self.scheduled_live_dependencies != expected_live_dependencies {
            return Err(invalid(
                "scheduled legal culture-dependency index is inconsistent",
            ));
        }
        for rule in &self.dirty_rules {
            if !self.rules.contains_key(rule) {
                return Err(invalid("legal dirty-rule index references an unknown rule"));
            }
        }
        for (id, case) in &self.cases {
            let forum = plan
                .forum_by_id
                .get(&case.forum)
                .and_then(|key| plan.forums.get(key.get() as usize));
            if id != &case.id
                || forum.is_none_or(|forum| {
                    !scope_covers(&forum.legal_orders, &case.legal_order)
                        || case
                            .subject_matters
                            .iter()
                            .any(|matter| !scope_covers(&forum.subject_matters, matter))
                        || !forum.proof_profiles.contains(&case.proof_profile)
                        || case
                            .standing
                            .as_ref()
                            .is_none_or(|standing| !forum.standing_profiles.contains(standing))
                        || case
                            .remedies
                            .iter()
                            .any(|remedy| !forum.remedy_profiles.contains(remedy))
                })
                || case.claims.is_empty()
                || case.issues.is_empty()
                || case.subject_matters.is_empty()
                || case.remedies.is_empty()
                || case.allegations.len() > plan.budgets.max_evidence_per_record
                || nested_case_items(case)
                    .is_none_or(|items| items > plan.budgets.max_nested_items_per_record)
            {
                return Err(invalid(
                    "legal case identity, forum, or evidence is invalid",
                ));
            }
        }
        for (id, finding) in &self.findings {
            if id != &finding.id
                || !self.cases.contains_key(&finding.case_id)
                || finding.evidence.len() > plan.budgets.max_evidence_per_record
                || self.cases.get(&finding.case_id).is_none_or(|case| {
                    !case.issues.contains(&finding.issue)
                        || finding.burden != case.proof_profile
                        || finding.at > case.deadline
                })
                || finding.predecessor.as_ref().is_some_and(|reference| {
                    self.findings.get(&reference.id).is_none_or(|predecessor| {
                        reference != &local_ref("finding", &predecessor.id)
                            || predecessor.case_id != finding.case_id
                            || predecessor.issue != finding.issue
                            || predecessor.at > finding.at
                    })
                })
            {
                return Err(invalid("legal finding topology is invalid"));
            }
        }
        for (id, ruling) in &self.rulings {
            if id != &ruling.id
                || !plan.institution_by_id.contains_key(&ruling.institution)
                || ruling.evidence.len() > plan.budgets.max_evidence_per_record
                || !self.cases.contains_key(&ruling.case_id)
                || !strictly_sorted(&ruling.issues)
                || !strictly_sorted(&ruling.findings)
                || (!ruling.sources.is_empty() && !strictly_sorted(&ruling.sources))
                || (!ruling.resolved_versions.is_empty()
                    && !strictly_sorted(&ruling.resolved_versions))
                || (!ruling.selected_versions.is_empty()
                    && !strictly_sorted(&ruling.selected_versions))
                || !strictly_sorted(&ruling.scope)
                || (!ruling.predecessors.is_empty() && !strictly_sorted(&ruling.predecessors))
                || (!ruling.evidence.is_empty() && !strictly_sorted(&ruling.evidence))
                || nested_ruling_items(ruling)
                    .is_none_or(|items| items > plan.budgets.max_nested_items_per_record)
                || ruling.findings.iter().any(|reference| {
                    self.findings
                        .get(&reference.id)
                        .is_none_or(|finding| reference != &local_ref("finding", &finding.id))
                })
                || ruling.sources.iter().any(|reference| {
                    self.sources
                        .get(&reference.id)
                        .is_none_or(|source| reference != &local_ref("source_version", &source.id))
                })
                || ruling.resolved_versions.iter().any(|reference| {
                    self.law_versions
                        .get(&reference.id)
                        .is_none_or(|version| reference != &law_version_reference(version))
                })
                || ruling
                    .selected_versions
                    .iter()
                    .any(|reference| ruling.resolved_versions.binary_search(reference).is_err())
                || ruling.predecessors.iter().any(|reference| {
                    self.rulings.get(&reference.id).is_none_or(|predecessor| {
                        reference != &local_ref("ruling", &predecessor.id)
                            || predecessor.case_id != ruling.case_id
                            || predecessor.effective_from > ruling.effective_from
                    })
                })
                || ruling
                    .effective_until
                    .is_some_and(|until| until < ruling.effective_from)
                || self.cases.get(&ruling.case_id).is_none_or(|case| {
                    let Some(forum) = plan
                        .forum_by_id
                        .get(&case.forum)
                        .and_then(|key| plan.forums.get(key.get() as usize))
                    else {
                        return true;
                    };
                    ruling.issues.is_empty()
                        || ruling.scope.is_empty()
                        || ruling.findings.is_empty()
                        || ruling
                            .issues
                            .iter()
                            .any(|issue| !case.issues.contains(issue))
                        || ruling.findings.iter().any(|reference| {
                            self.findings.get(&reference.id).is_none_or(|finding| {
                                finding.case_id != case.id
                                    || !ruling.issues.contains(&finding.issue)
                            })
                        })
                        || !forum.institutions.contains(&ruling.institution)
                        || ruling.remedy.as_ref().is_some_and(|remedy| {
                            !case.remedies.contains(remedy)
                                || !forum.remedy_profiles.contains(remedy)
                        })
                        || ruling
                            .precedent_profile
                            .as_ref()
                            .is_some_and(|precedent| !forum.precedent_profiles.contains(precedent))
                        || plan
                            .institutions
                            .iter()
                            .find(|institution| institution.id == ruling.institution)
                            .is_none_or(|institution| {
                                !institution.competences.iter().any(|competence| {
                                    competence.can_adjudicate
                                        && scope_covers(&competence.forums, &case.forum)
                                        && scope_covers(&competence.legal_orders, &case.legal_order)
                                        && scope_covers(
                                            &competence.jurisdictions,
                                            &forum.jurisdiction,
                                        )
                                        && case.subject_matters.iter().all(|matter| {
                                            scope_covers(&competence.subject_matters, matter)
                                        })
                                        && ruling.scope.iter().all(|scope| {
                                            scope_covers(&competence.jurisdictions, scope)
                                        })
                                })
                            })
                })
            {
                return Err(invalid("legal ruling topology is invalid"));
            }
        }
        for (id, conflict) in &self.conflicts {
            let mut partition = conflict.governing_versions.clone();
            partition.extend(conflict.displaced_versions.iter().cloned());
            partition.sort();
            partition.dedup();
            let basis_is_authorized = conflict.versions.iter().all(|reference| {
                self.law_versions
                    .get(&reference.id)
                    .and_then(|version| self.rules.get(&version.rule))
                    .and_then(|rule| plan.order_by_id.get(&rule.legal_order))
                    .and_then(|key| plan.orders.get(key.get() as usize))
                    .and_then(|order| plan.precedence_profile_by_id.get(&order.precedence_profile))
                    .and_then(|key| plan.precedence_profiles.get(key.get() as usize))
                    .is_some_and(|profile| profile.ordered_bases.contains(&conflict.basis))
            });
            let temporal_winner = conflict.versions.iter().max_by_key(|reference| {
                self.law_versions.get(&reference.id).map(|version| {
                    (
                        version.effective_at,
                        version.adopted_at,
                        version.legal_ordinal,
                        version.id.as_str(),
                    )
                })
            });
            if id != &conflict.id
                || conflict.versions.len() < 2
                || conflict.versions.len() > plan.budgets.max_applicability_query_work
                || nested_conflict_items(conflict)
                    .is_none_or(|items| items > plan.budgets.max_nested_items_per_record)
                || !strictly_sorted(&conflict.versions)
                || (!conflict.governing_versions.is_empty()
                    && !strictly_sorted(&conflict.governing_versions))
                || (!conflict.displaced_versions.is_empty()
                    && !strictly_sorted(&conflict.displaced_versions))
                || (!conflict.trace.is_empty() && !strictly_sorted(&conflict.trace))
                || conflict.rationale.trim().is_empty()
                || conflict.recorded_at > self.last_settled_at
                || conflict
                    .effective_until
                    .is_some_and(|until| until < conflict.effective_from)
                || conflict
                    .jurisdiction
                    .as_ref()
                    .is_some_and(|jurisdiction| !plan.jurisdiction_by_id.contains_key(jurisdiction))
                || !basis_is_authorized
                || conflict.versions.iter().any(|reference| {
                    self.law_versions
                        .get(&reference.id)
                        .is_none_or(|version| reference != &law_version_reference(version))
                })
                || conflict.ruling.as_ref().is_some_and(|reference| {
                    self.rulings
                        .get(&reference.id)
                        .is_none_or(|ruling| reference != &local_ref("ruling", &ruling.id))
                })
                || match conflict.resolution {
                    ApplicabilityOutcome::Displaced => {
                        conflict.governing_versions.is_empty()
                            || conflict.displaced_versions.is_empty()
                            || partition != conflict.versions
                            || conflict
                                .governing_versions
                                .iter()
                                .any(|reference| conflict.displaced_versions.contains(reference))
                    }
                    ApplicabilityOutcome::Contested => {
                        !conflict.governing_versions.is_empty()
                            || !conflict.displaced_versions.is_empty()
                    }
                    _ => true,
                }
                || (conflict.basis == ConflictResolutionBasis::Temporal
                    && conflict.resolution == ApplicabilityOutcome::Displaced
                    && temporal_winner.is_none_or(|winner| {
                        conflict.governing_versions.as_slice() != [winner.clone()]
                    }))
                || (conflict.resolution == ApplicabilityOutcome::Displaced
                    && conflict.basis != ConflictResolutionBasis::Temporal
                    && conflict.ruling.is_none())
                || conflict.ruling.as_ref().is_some_and(|reference| {
                    self.rulings.get(&reference.id).is_none_or(|ruling| {
                        ruling.disposition != OperativeDisposition::Operative
                            || ruling.resolved_versions != conflict.versions
                            || ruling.selected_versions != conflict.governing_versions
                            || ruling.effective_from > conflict.effective_from
                            || match (ruling.effective_until, conflict.effective_until) {
                                (Some(ruling_until), Some(conflict_until)) => {
                                    ruling_until < conflict_until
                                }
                                (Some(_), None) => true,
                                (None, _) => false,
                            }
                            || conflict.jurisdiction.as_ref().is_some_and(|jurisdiction| {
                                !scope_covers(&ruling.scope, jurisdiction)
                            })
                            || (conflict.basis != ConflictResolutionBasis::Temporal
                                && conflict.jurisdiction.is_none())
                    })
                })
            {
                return Err(invalid("legal conflict topology is invalid"));
            }
        }
        let mut expected_conflicts_by_version = BTreeMap::<String, BTreeSet<String>>::new();
        for conflict in self.conflicts.values() {
            for reference in &conflict.versions {
                expected_conflicts_by_version
                    .entry(reference.id.clone())
                    .or_default()
                    .insert(conflict.id.clone());
            }
        }
        if self.conflict_ids_by_version != expected_conflicts_by_version {
            return Err(invalid("legal conflict query index is inconsistent"));
        }
        for succession in &self.successions {
            if succession.evidence.len() > plan.budgets.max_evidence_per_record
                || succession.reception.is_empty()
                || succession
                    .reception
                    .iter()
                    .any(|rule| !valid_reception_rule(rule))
                || succession.reception.iter().any(|rule| {
                    rule.transform.as_ref().is_some_and(|transform| {
                        !plan.clauses.iter().any(|clause| &clause.id == transform)
                    })
                })
                || succession
                    .reception
                    .windows(2)
                    .any(|pair| pair[0].rule_prefix >= pair[1].rule_prefix)
                || !strictly_sorted(&succession.predecessors)
                || !strictly_sorted(&succession.successors)
                || !strictly_sorted(&succession.territorial_scope)
                || !strictly_sorted(&succession.personal_scope)
                || succession
                    .predecessors
                    .iter()
                    .chain(&succession.successors)
                    .any(|order| !plan.order_by_id.contains_key(order))
            {
                return Err(invalid("legal succession topology is invalid"));
            }
        }
        let mut expected_retired_culture = BTreeSet::new();
        let mut retirement_ids = BTreeSet::new();
        for retirement in &self.retirements {
            if retirement.evidence.len() > plan.budgets.max_evidence_per_record
                || !retirement_ids.insert(retirement.id.as_str())
            {
                return Err(invalid("legal retirement identity or evidence is invalid"));
            }
            if retirement.kind == "culture_target" {
                let target = retirement
                    .cultural_target
                    .as_ref()
                    .ok_or_else(|| invalid("culture retirement lacks an exact generation"))?;
                let record_id = format!("{}@{}", target.target, target.generation);
                if retirement.record
                    != DomainRecordRef::new(PLUGIN_NAMESPACE, "cultural_target", &record_id)
                    || !expected_retired_culture.insert(target.clone())
                {
                    return Err(invalid("culture retirement generation is inconsistent"));
                }
            }
        }
        if self.retired_cultural_targets != expected_retired_culture {
            return Err(invalid("retired culture generation index is inconsistent"));
        }
        let mut expected_successions_by_successor = BTreeMap::<String, Vec<usize>>::new();
        for (index, succession) in self.successions.iter().enumerate() {
            for successor in &succession.successors {
                expected_successions_by_successor
                    .entry(successor.clone())
                    .or_default()
                    .push(index);
            }
        }
        if self.succession_indexes_by_successor != expected_successions_by_successor {
            return Err(invalid("legal succession query index is inconsistent"));
        }
        let mut indexed_applicability = BTreeSet::new();
        for (rule_id, keys) in &self.applicability_by_rule {
            if !self.rules.contains_key(rule_id) {
                return Err(invalid(
                    "legal applicability index references an unknown rule",
                ));
            }
            for key in keys {
                let result = self
                    .applicability
                    .get(key)
                    .ok_or_else(|| invalid("legal applicability projection is missing"))?;
                if !indexed_applicability.insert(key.clone())
                    || result.versions.iter().any(|reference| {
                        self.law_versions
                            .get(&reference.id)
                            .is_none_or(|version| version.rule != *rule_id)
                    })
                {
                    return Err(invalid("legal applicability index is inconsistent"));
                }
            }
        }
        if indexed_applicability != self.applicability.keys().cloned().collect() {
            return Err(invalid("legal applicability index is incomplete"));
        }
        Ok(())
    }

    #[must_use]
    pub fn snapshot_state(&self) -> Self {
        self.clone()
    }
    #[must_use]
    pub fn pending_outbox(&self) -> impl Iterator<Item = &LegalDecisionOutboxItem> {
        self.pending_outbox_sequences
            .iter()
            .filter_map(|sequence| self.outbox.get(sequence))
    }
    #[must_use]
    pub fn operative_rules(&self) -> impl Iterator<Item = &LegalRule> {
        self.rules
            .values()
            .filter(|rule| !rule.retired && rule.operative_version.is_some())
    }

    /// Submit a proposal and freeze its procedure roster at this boundary.
    pub fn submit_proposal(
        &mut self,
        plan: &CompiledLawPlan,
        proposal: LegalProposal,
    ) -> Result<(), CanwuError> {
        self.validate_against_plan(plan)?;
        self.submit_proposal_within_boundary(plan, proposal)
    }

    pub(crate) fn submit_proposal_within_boundary(
        &mut self,
        plan: &CompiledLawPlan,
        mut proposal: LegalProposal,
    ) -> Result<(), CanwuError> {
        validate_proposal_fanout(plan, &proposal)?;
        validate_runtime_text_budget(&proposal, plan.budgets.max_text_bytes)?;
        if proposal
            .cultural_dependencies
            .iter()
            .any(|dependency| self.retired_cultural_targets.contains(&dependency.target))
        {
            return Err(invalid(
                "legal proposal depends on a retired culture generation",
            ));
        }
        if self.proposals.len() >= plan.budgets.max_procedures {
            return Err(invalid("legal proposal budget exhausted"));
        }
        if self.proposals.contains_key(&proposal.id) {
            return Err(invalid(format!("duplicate legal proposal {}", proposal.id)));
        }
        self.validate_proposal_rule_transition(&proposal)?;
        let source_profile = plan
            .source_profile_by_id
            .get(&proposal.source_profile)
            .and_then(|key| plan.source_profiles.get(key.get() as usize))
            .ok_or_else(|| invalid("proposal references an unknown source profile"))?;
        if proposal.evidence.len() < source_profile.min_evidence
            || proposal.evidence.len() > source_profile.max_evidence
            || (source_profile.require_claimant && proposal.sponsor.is_none())
        {
            return Err(invalid(
                "proposal does not satisfy its compiled source admission contract",
            ));
        }
        self.validate_proposal_legal_claim(plan, source_profile, &proposal)?;
        if source_profile.procedure.as_deref() != Some(proposal.procedure_profile.as_str())
            || !plan.order_by_id.contains_key(&proposal.legal_order)
            || !plan
                .procedure_by_id
                .contains_key(&proposal.procedure_profile)
            || proposal
                .jurisdictions
                .iter()
                .any(|id| !plan.jurisdiction_by_id.contains_key(id))
        {
            return Err(invalid(
                "proposal source profile does not authorize its legal order or procedure",
            ));
        }
        for clause in &proposal.clauses {
            if !plan
                .clauses
                .iter()
                .any(|candidate| candidate.id == clause.clause)
            {
                return Err(invalid(format!("unknown clause {}", clause.clause)));
            }
        }
        canonicalize_proposal(&mut proposal);
        proposal.admitted_signal_kinds.clear();
        proposal.evidence.sort();
        proposal.evidence.dedup();
        proposal.expected_versions.sort();
        proposal.expected_versions.dedup();
        if proposal.evidence.len() > plan.budgets.max_evidence_per_record {
            return Err(invalid("legal proposal evidence budget exhausted"));
        }
        proposal
            .procedure_profile_hash
            .clone_from(&plan.content_hash);
        proposal.status = ProposalStatus::Submitted;
        let profile = plan
            .procedure_by_id
            .get(&proposal.procedure_profile)
            .and_then(|key| plan.procedures.get(key.get() as usize))
            .ok_or_else(|| invalid("legal proposal procedure profile is missing"))?;
        let seats = profile
            .stages
            .first()
            .map_or_else(Vec::new, |stage| stage.seats.clone());
        let all_seats = profile
            .stages
            .iter()
            .flat_map(|stage| &stage.seats)
            .cloned()
            .collect::<BTreeSet<_>>();
        let seat_authorities = all_seats
            .iter()
            .map(|seat| {
                let (holder, permission_profile_id, decision_controller_id) =
                    seat_authority(plan, &profile.source_id, seat);
                (
                    seat.clone(),
                    ProcedureSeatAuthority {
                        holder,
                        permission_profile_id,
                        decision_controller_id,
                    },
                )
            })
            .collect();
        let procedure_id = format!("procedure:{}", proposal.id);
        let instance = ProcedureInstance {
            id: procedure_id.clone(),
            proposal: local_ref("proposal", &proposal.id),
            profile: proposal.procedure_profile.clone(),
            profile_hash: plan.content_hash.clone(),
            stages: profile.stages.clone(),
            active_stage: 0,
            round: 0,
            eligible_seats: seats,
            seat_authorities,
            deadline: proposal.deadline,
            evidence: proposal.evidence.clone(),
            closed: false,
        };
        proposal.active_procedure = Some(procedure_id.clone());
        let proposal_dependencies = Self::proposal_identity_evidence_dependencies(&proposal);
        let procedure_dependencies = instance.evidence.iter().cloned().collect();
        self.reserve_state_growth(Self::encoded_growth(&(&proposal, &instance), 8)?)?;
        self.proposals.insert(proposal.id.clone(), proposal);
        self.open_procedures.insert(procedure_id.clone());
        self.dirty_procedures.insert(procedure_id.clone());
        self.procedures_by_deadline
            .entry(procedure_expiry_time(instance.deadline)?)
            .or_default()
            .insert(procedure_id.clone());
        self.procedures.insert(procedure_id, instance);
        self.add_identity_evidence_component(proposal_dependencies)?;
        self.add_identity_evidence_component(procedure_dependencies)?;
        Ok(())
    }

    fn validate_proposal_rule_transition(
        &self,
        proposal: &LegalProposal,
    ) -> Result<(), CanwuError> {
        let rule = self.rules.get(&proposal.rule_id);
        match proposal.operation {
            LawOperation::Establish | LawOperation::Recognize | LawOperation::Receive => {
                if rule.is_some() || proposal.expected_rule_head.is_some() {
                    return Err(invalid(
                        "rule-creating operation requires an absent rule and empty head guard",
                    ));
                }
            }
            LawOperation::Amend
            | LawOperation::Suspend
            | LawOperation::Resume
            | LawOperation::Displace
            | LawOperation::Annul
            | LawOperation::Repeal
            | LawOperation::Expire => {
                let rule = rule
                    .ok_or_else(|| invalid("rule-changing operation requires an existing rule"))?;
                if rule.legal_order != proposal.legal_order
                    || proposal.expected_rule_head != rule.latest_adopted_version
                {
                    return Err(invalid(
                        "rule-changing operation failed legal-order or head compare-and-set",
                    ));
                }
                let latest = rule
                    .latest_adopted_version
                    .as_ref()
                    .and_then(|reference| self.law_versions.get(&reference.id))
                    .ok_or_else(|| invalid("rule-changing operation has no exact current head"))?;
                if proposal.effective_at < latest.effective_at {
                    return Err(invalid(
                        "rule-changing operation cannot precede its current head effective time",
                    ));
                }
                let operative = rule
                    .operative_version
                    .as_ref()
                    .and_then(|reference| self.law_versions.get(&reference.id));
                let allowed = match proposal.operation {
                    LawOperation::Amend => {
                        (operative.is_some_and(|version| {
                            version.disposition == OperativeDisposition::Operative
                        }) || (operative.is_none()
                            && matches!(
                                latest.disposition,
                                OperativeDisposition::Claimed
                                    | OperativeDisposition::Purported
                                    | OperativeDisposition::Contested
                            )
                            && proposal.validity == OperativeDisposition::Operative))
                            && !rule.retired
                    }
                    LawOperation::Suspend => {
                        operative.is_some_and(|version| {
                            version.disposition == OperativeDisposition::Operative
                        }) && !rule.retired
                    }
                    LawOperation::Resume => {
                        operative.is_some_and(|version| {
                            version.disposition == OperativeDisposition::Suspended
                        }) && !rule.retired
                    }
                    LawOperation::Displace
                    | LawOperation::Annul
                    | LawOperation::Repeal
                    | LawOperation::Expire => !rule.retired,
                    LawOperation::Establish | LawOperation::Recognize | LawOperation::Receive => {
                        false
                    }
                };
                if !allowed {
                    return Err(invalid("illegal rule operation state transition"));
                }
            }
        }
        Ok(())
    }

    fn validate_adoption_guards(
        &self,
        plan: &CompiledLawPlan,
        proposal: &LegalProposal,
        at: SimTime,
    ) -> Result<(), CanwuError> {
        let profile = plan
            .source_profile_by_id
            .get(&proposal.source_profile)
            .and_then(|key| plan.source_profiles.get(key.get() as usize))
            .ok_or_else(|| invalid("legal source profile is missing"))?;
        let publicity = proposal
            .publicity
            .as_ref()
            .map(|reference| {
                self.publicity_events
                    .get(&reference.id)
                    .filter(|event| {
                        reference == &local_ref("publicity", &event.id)
                            && event.proposal == local_ref("proposal", &proposal.id)
                    })
                    .ok_or_else(|| invalid("legal proposal publicity event is not exact"))
            })
            .transpose()?;
        match profile.publicity_policy {
            PublicityPolicy::ValidityCondition if publicity.is_none_or(|event| event.at > at) => {
                return Err(invalid(
                    "legal validity requires publicity no later than adoption",
                ));
            }
            PublicityPolicy::EffectivenessCondition
                if publicity.is_some_and(|event| event.at > proposal.effective_at) =>
            {
                return Err(invalid(
                    "legal effectiveness requires publicity no later than effective time",
                ));
            }
            PublicityPolicy::NotRequired if publicity.is_some() => {
                return Err(invalid(
                    "legal source profile does not admit a publicity event",
                ));
            }
            _ => {}
        }
        if proposal.effective_at < at {
            if !profile.allow_retroactive
                || proposal
                    .retrospective_from
                    .is_none_or(|from| from > proposal.effective_at)
            {
                return Err(invalid(
                    "retroactive legal effect requires an authorized retrospective date",
                ));
            }
        } else if proposal
            .retrospective_from
            .is_some_and(|from| !profile.allow_retroactive || from > proposal.effective_at)
        {
            return Err(invalid("legal retrospective date is not authorized"));
        }
        Ok(())
    }

    fn validate_proposal_legal_claim(
        &self,
        plan: &CompiledLawPlan,
        profile: &CompiledSourceProfile,
        proposal: &LegalProposal,
    ) -> Result<(), CanwuError> {
        if !legal_claim_fields_match(proposal.competence, &proposal.defects, proposal.validity) {
            return Err(invalid(
                "legal competence, defects, and validity claim are inconsistent",
            ));
        }
        if (proposal.operation == LawOperation::Receive) != (profile.mode == SourceMode::Received) {
            return Err(invalid(
                "received-law operations require exactly a received source profile",
            ));
        }
        if profile.procedure.is_some()
            && !institutional_competence_covers(plan, profile, proposal)
            && (proposal.competence == LegalCompetenceDisposition::Confirmed
                || !proposal
                    .defects
                    .iter()
                    .any(|defect| defect == "institutional_competence_unproven"))
        {
            return Err(invalid(
                "proposal is outside compiled institutional competence",
            ));
        }
        let origin_matches = match (profile.origin_policy, proposal.origin.as_ref()) {
            (SourceOriginPolicy::NoOrigin, None) => true,
            (SourceOriginPolicy::Ruling, Some(LegalOriginRef::Ruling { ruling })) => {
                self.rulings.get(&ruling.id).is_some_and(|record| {
                    ruling == &local_ref("ruling", &record.id)
                        && record.disposition == OperativeDisposition::Operative
                        && record.effective_from <= proposal.effective_at
                        && record
                            .effective_until
                            .is_none_or(|until| proposal.effective_at <= until)
                        && proposal
                            .jurisdictions
                            .iter()
                            .all(|jurisdiction| scope_covers(&record.scope, jurisdiction))
                })
            }
            (
                SourceOriginPolicy::Agreement,
                Some(LegalOriginRef::Agreement {
                    instrument,
                    parties,
                    ratifications,
                }),
            ) => {
                proposal
                    .expected_versions
                    .iter()
                    .any(|expected| expected == instrument)
                    && proposal
                        .evidence
                        .contains(&EvidenceRef::DomainRecordVersion(instrument.clone()))
                    && profile.agreement_namespace.as_ref()
                        == Some(&instrument.record.kind.namespace)
                    && profile.agreement_kind.as_ref() == Some(&instrument.record.kind.name)
                    && strictly_sorted(parties)
                    && parties.len() >= profile.min_agreement_parties
                    && proposal
                        .sponsor
                        .as_ref()
                        .is_some_and(|sponsor| parties.contains(sponsor))
                    && (!profile.require_agreement_ratification || !ratifications.is_empty())
                    && strictly_sorted(ratifications)
                    && ratifications
                        .iter()
                        .all(|ratification| proposal.evidence.contains(ratification))
            }
            (SourceOriginPolicy::Reception, Some(origin @ LegalOriginRef::Reception { .. })) => {
                self.reception_origin_matches(
                    plan,
                    origin,
                    &proposal.legal_order,
                    proposal.effective_at,
                    proposal,
                )
            }
            _ => false,
        };
        if !origin_matches {
            return Err(invalid(
                "legal source mode lacks its exact ruling, agreement, or reception origin",
            ));
        }
        Ok(())
    }

    fn reception_origin_matches(
        &self,
        plan: &CompiledLawPlan,
        origin: &LegalOriginRef,
        successor_order: &str,
        effective_at: SimTime,
        proposal: &LegalProposal,
    ) -> bool {
        let LegalOriginRef::Reception {
            succession: succession_id,
            predecessor,
            transform,
        } = origin
        else {
            return false;
        };
        let origin_transform = transform.as_deref();
        let Some(version) = self.law_versions.get(&predecessor.id) else {
            return false;
        };
        let Some(rule) = self.rules.get(&version.rule) else {
            return false;
        };
        let Some(succession) = self
            .successions
            .iter()
            .find(|candidate| &candidate.id == succession_id)
        else {
            return false;
        };
        predecessor == &law_version_reference(version)
            && succession.effective_at <= effective_at
            && succession.predecessors.contains(&rule.legal_order)
            && succession
                .successors
                .iter()
                .any(|order| order == successor_order)
            && matching_reception_rule(&succession.reception, &rule.id).is_some_and(|reception| {
                match reception.action {
                    ReceptionAction::Transform => {
                        reception.transform.as_deref() == origin_transform
                            && origin_transform.is_some_and(|target| {
                                plan.clauses.iter().any(|clause| clause.id == target)
                                    && !proposal.clauses.is_empty()
                                    && proposal
                                        .clauses
                                        .iter()
                                        .all(|clause| clause.clause == target)
                            })
                    }
                    ReceptionAction::Review => origin_transform.is_none(),
                    ReceptionAction::Continue | ReceptionAction::Displace => false,
                }
            })
    }

    /// Returns capacity requests that the host should submit to the existing
    /// deterministic reservation phase.
    pub fn pending_capacity_requirements(
        &self,
        plan: &CompiledLawPlan,
    ) -> Result<Vec<LegalCapacityRequirement>, CanwuError> {
        if self.plan_hash != plan.content_hash {
            return Err(invalid("legal runtime plan hash mismatch"));
        }
        let mut requirements = Vec::new();
        for procedure_id in &self.open_procedures {
            if self.capacity_allocations.contains_key(procedure_id) {
                continue;
            }
            let procedure = self
                .procedures
                .get(procedure_id)
                .ok_or_else(|| invalid("open legal procedure is missing"))?;
            let profile = plan
                .procedure_by_id
                .get(&procedure.profile)
                .and_then(|key| plan.procedures.get(key.get() as usize))
                .ok_or_else(|| invalid("legal procedure profile is missing"))?;
            if let Some(pool) = &profile.reservation_pool {
                requirements.push(LegalCapacityRequirement {
                    procedure: procedure_id.clone(),
                    pool: pool.clone(),
                    quantity: profile.reservation_quantity,
                });
            }
        }
        Ok(requirements)
    }

    /// Admit one exact allocation returned by the host's reservation phase.
    pub fn admit_capacity_allocation(
        &mut self,
        plan: &CompiledLawPlan,
        allocation: LegalCapacityAllocation,
    ) -> Result<(), CanwuError> {
        validate_runtime_text_budget(&allocation, plan.budgets.max_text_bytes)?;
        if self.plan_hash != plan.content_hash {
            return Err(invalid("legal runtime plan hash mismatch"));
        }
        let procedure = self
            .procedures
            .get(&allocation.procedure)
            .filter(|procedure| !procedure.closed)
            .ok_or_else(|| invalid("legal capacity allocation targets no open procedure"))?;
        let profile = plan
            .procedure_by_id
            .get(&procedure.profile)
            .and_then(|key| plan.procedures.get(key.get() as usize))
            .ok_or_else(|| invalid("legal procedure profile is missing"))?;
        if profile.reservation_pool.as_ref() != Some(&allocation.pool)
            || profile.reservation_quantity != allocation.quantity
        {
            return Err(invalid(
                "legal capacity allocation does not match its compiled requirement",
            ));
        }
        if let Some(existing) = self.capacity_allocations.get(&allocation.procedure) {
            return if existing == &allocation {
                Ok(())
            } else {
                Err(invalid("conflicting legal capacity allocation"))
            };
        }
        let procedure_id = procedure.id.clone();
        self.reserve_state_growth(Self::encoded_growth(&allocation, 4)?)?;
        self.capacity_allocations
            .insert(allocation.procedure.clone(), allocation);
        self.dirty_procedures.insert(procedure_id);
        Ok(())
    }

    /// Admit a compiled source profile that intentionally has no controller.
    ///
    /// The host calls this inside an authoritative legal boundary after it has
    /// admitted the exact evidence. No synthetic decision ticket is created.
    pub fn admit_non_procedural_source(
        &mut self,
        plan: &CompiledLawPlan,
        proposal: LegalProposal,
        admitted_signal_kinds: &[String],
        at: SimTime,
    ) -> Result<(), CanwuError> {
        self.validate_against_plan(plan)?;
        self.admit_non_procedural_source_bound(plan, proposal, admitted_signal_kinds, at)
    }

    fn admit_non_procedural_source_bound(
        &mut self,
        plan: &CompiledLawPlan,
        mut proposal: LegalProposal,
        admitted_signal_kinds: &[String],
        at: SimTime,
    ) -> Result<(), CanwuError> {
        validate_proposal_fanout(plan, &proposal)?;
        validate_runtime_text_budget(&proposal, plan.budgets.max_text_bytes)?;
        validate_runtime_text_budget(admitted_signal_kinds, plan.budgets.max_text_bytes)?;
        if proposal
            .cultural_dependencies
            .iter()
            .any(|dependency| self.retired_cultural_targets.contains(&dependency.target))
        {
            return Err(invalid(
                "legal source depends on a retired culture generation",
            ));
        }
        if at < self.last_settled_at {
            return Err(invalid("non-procedural legal admission is in the past"));
        }
        let profile = plan
            .source_profile_by_id
            .get(&proposal.source_profile)
            .and_then(|key| plan.source_profiles.get(key.get() as usize))
            .ok_or_else(|| invalid("non-procedural source profile is missing"))?;
        if profile.procedure.is_some() {
            return Err(invalid(
                "a controller-bound source profile must use the procedure and outbox path",
            ));
        }
        self.validate_proposal_legal_claim(plan, profile, &proposal)?;
        if proposal.id.is_empty()
            || proposal.rule_id.is_empty()
            || self.proposals.contains_key(&proposal.id)
            || !plan.order_by_id.contains_key(&proposal.legal_order)
            || proposal.evidence.is_empty()
            || proposal.evidence.len() < profile.min_evidence
            || proposal.evidence.len() > profile.max_evidence
            || (profile.require_claimant && proposal.sponsor.is_none())
            || self.proposals.len() >= plan.budgets.max_procedures
            || self.sources.len() >= plan.budgets.max_sources
            || self.law_versions.len() >= plan.budgets.max_law_versions
            || (!self.rules.contains_key(&proposal.rule_id)
                && self.rules.len() >= plan.budgets.max_rules)
        {
            return Err(invalid(
                "non-procedural legal source violates identity, evidence, or state budgets",
            ));
        }
        if proposal
            .jurisdictions
            .iter()
            .any(|id| !plan.jurisdiction_by_id.contains_key(id))
            || proposal.clauses.iter().any(|clause| {
                !plan
                    .clauses
                    .iter()
                    .any(|candidate| candidate.id == clause.clause)
            })
        {
            return Err(invalid(
                "non-procedural legal source references unknown scope or clauses",
            ));
        }
        let kinds = admitted_signal_kinds
            .iter()
            .filter(|kind| !kind.is_empty())
            .cloned()
            .collect::<BTreeSet<_>>();
        if profile
            .required_signal_kinds
            .iter()
            .any(|kind| !kinds.contains(kind))
        {
            return Err(invalid(
                "non-procedural legal source is missing required admitted signal kinds",
            ));
        }
        canonical_hash("canwu.law.source.v1", &proposal.clauses)?;
        self.validate_proposal_rule_transition(&proposal)?;
        self.validate_adoption_guards(plan, &proposal, at)?;
        self.next_source_ordinal
            .checked_add(1)
            .ok_or_else(|| invalid("legal source ordinal overflowed"))?;
        let next_boundary = self
            .boundary_index
            .checked_add(1)
            .ok_or_else(|| invalid("legal boundary index overflowed"))?;

        proposal.procedure_profile.clear();
        proposal
            .procedure_profile_hash
            .clone_from(&plan.content_hash);
        proposal.active_procedure = None;
        proposal.status = ProposalStatus::Submitted;
        proposal.admitted_signal_kinds = kinds;
        canonicalize_proposal(&mut proposal);
        proposal.evidence.sort();
        proposal.evidence.dedup();
        proposal.expected_versions.sort();
        proposal.expected_versions.dedup();
        self.reserve_state_growth(Self::encoded_growth(&proposal, 32)?)?;
        let proposal_id = proposal.id.clone();
        let proposal_dependencies = Self::proposal_identity_evidence_dependencies(&proposal);
        self.boundary_index = next_boundary;
        self.last_settled_at = at;
        self.proposals.insert(proposal_id.clone(), proposal);
        self.add_identity_evidence_component(proposal_dependencies)?;
        self.adopt_proposal_record(plan, &proposal_id, None, at)?;
        Ok(())
    }

    pub(crate) fn admit_non_procedural_source_within_boundary(
        &mut self,
        plan: &CompiledLawPlan,
        proposal: LegalProposal,
        admitted_signal_kinds: &[String],
        at: SimTime,
    ) -> Result<(), CanwuError> {
        let boundary_index = self.boundary_index;
        let last_settled_at = self.last_settled_at;
        self.admit_non_procedural_source_bound(plan, proposal, admitted_signal_kinds, at)?;
        self.boundary_index = boundary_index;
        self.last_settled_at = last_settled_at;
        Ok(())
    }

    /// Accept an authorized command's bounded pending intent. No legal record is mutated here.
    pub fn queue_pending_intent(
        &mut self,
        plan: &CompiledLawPlan,
        intent: PendingLegalIntent,
    ) -> Result<(), CanwuError> {
        self.validate_against_plan(plan)?;
        self.queue_authorized_pending_intent(intent)
    }

    pub(crate) fn queue_authorized_pending_intent(
        &mut self,
        intent: PendingLegalIntent,
    ) -> Result<(), CanwuError> {
        validate_runtime_text_budget(&intent, self.budgets.max_text_bytes)?;
        if self.pending_intents.len() >= self.budgets.max_pending_intents {
            return Err(invalid("pending legal intent budget exhausted"));
        }
        if self.consumed_intent_ids.contains(&intent.id) {
            return Ok(());
        }
        if self.pending_intents.contains_key(&intent.id) {
            return Err(invalid("duplicate pending legal intent"));
        }
        self.validate_intent_binding(&intent)?;
        self.reserve_state_growth(Self::encoded_growth(&intent, 8)?)?;
        self.dirty_procedures.insert(intent.procedure.id.clone());
        self.pending_intents.insert(intent.id.clone(), intent);
        Ok(())
    }

    fn validate_intent_binding(&self, intent: &PendingLegalIntent) -> Result<(), CanwuError> {
        let procedure = self
            .procedures
            .get(&intent.procedure.id)
            .ok_or_else(|| invalid("pending intent references unknown procedure"))?;
        let proposal = self
            .proposals
            .get(&intent.proposal.id)
            .ok_or_else(|| invalid("pending intent references unknown proposal"))?;
        if procedure.closed
            || intent.proposal != procedure.proposal
            || intent.procedure != procedure_reference(procedure)
            || intent.stage != procedure.active_stage
            || intent.round != procedure.round
        {
            return Err(invalid(
                "pending intent does not target the exact active procedure version",
            ));
        }
        let stage = procedure
            .stages
            .get(procedure.active_stage)
            .ok_or_else(|| invalid("procedure stage index is invalid"))?;
        if !stage.seats.contains(&intent.seat) {
            return Err(invalid("pending intent seat is not eligible"));
        }
        if intent.expected_versions != proposal.expected_versions
            || intent.intended_effective_at != proposal.effective_at
            || intent.clause_hash
                != canonical_hash("canwu.law.proposal-clauses.v1", &proposal.clauses)?
        {
            return Err(invalid(
                "pending intent authority or expected legal state does not match",
            ));
        }
        let key = outbox_key(
            &procedure.id,
            procedure.active_stage,
            procedure.round,
            &intent.seat,
        );
        let outbox = self
            .outbox_sequence_by_key
            .get(&key)
            .and_then(|sequence| self.outbox.get(sequence))
            .ok_or_else(|| invalid("pending intent has no originating legal outbox item"))?;
        if intent.request_id != Some(outbox.nested_command_request_id)
            || outbox.controller != intent.controller
            || outbox.proposal != intent.proposal
            || outbox.procedure != intent.procedure
            || !matches!(
                outbox.dispatch,
                DispatchState::Enqueued | DispatchState::Acknowledged
            )
        {
            return Err(invalid(
                "pending intent does not match an enqueued legal decision",
            ));
        }
        Ok(())
    }

    pub(crate) fn mark_outbox_enqueued(
        &mut self,
        sequence: u64,
        expected_revision: u64,
        ingress: EvidenceRef,
        outcome_commitment: String,
    ) -> Result<(), CanwuError> {
        let item = self
            .outbox
            .get(&sequence)
            .ok_or_else(|| invalid("unknown legal outbox sequence"))?;
        if !matches!(ingress, EvidenceRef::Ingress(_)) || outcome_commitment.is_empty() {
            return Err(invalid(
                "legal outbox enqueue acknowledgement requires ingress evidence",
            ));
        }
        if item.dispatch == DispatchState::Pending
            && item.enqueue_expected_revision == Some(expected_revision)
        {
            self.reserve_state_growth(512)?;
            let item = self
                .outbox
                .get_mut(&sequence)
                .ok_or_else(|| invalid("validated legal outbox sequence disappeared"))?;
            item.dispatch = DispatchState::Enqueued;
            item.enqueue_ingress = Some(ingress);
            item.enqueue_outcome_commitment = Some(outcome_commitment);
            self.pending_outbox_sequences.remove(&sequence);
        } else if item.enqueue_expected_revision != Some(expected_revision)
            || item.enqueue_ingress.as_ref() != Some(&ingress)
            || item.enqueue_outcome_commitment.as_deref() != Some(&outcome_commitment)
        {
            return Err(invalid(
                "legal outbox enqueue acknowledgement conflicts with its persisted identity",
            ));
        }
        Ok(())
    }

    /// Stage an expected revision inside a runtime that the host will persist
    /// before any decision ingress is enqueued.
    pub(crate) fn stage_outbox_expected_revision(
        &mut self,
        sequence: u64,
        expected_revision: u64,
    ) -> Result<(), CanwuError> {
        let item = self
            .outbox
            .get(&sequence)
            .ok_or_else(|| invalid("unknown legal outbox sequence"))?;
        if item.dispatch != DispatchState::Pending {
            return if item.enqueue_expected_revision == Some(expected_revision) {
                Ok(())
            } else {
                Err(invalid("legal outbox is no longer pending preparation"))
            };
        }
        if item.enqueue_expected_revision == Some(expected_revision) {
            return Ok(());
        }
        self.reserve_state_growth(64)?;
        let item = self
            .outbox
            .get_mut(&sequence)
            .ok_or_else(|| invalid("validated legal outbox sequence disappeared"))?;
        item.enqueue_expected_revision = Some(expected_revision);
        Ok(())
    }

    /// Stage a bounded holder-relative knowledge projection admitted by the plugin.
    pub(crate) fn stage_actor_context(
        &mut self,
        requirement: &LegalActorContextRequirement,
        context: LegalActorContext,
    ) -> Result<(), CanwuError> {
        validate_runtime_text_budget(&context, self.budgets.max_text_bytes)?;
        if context.knowledge_record_ids.len() > self.budgets.max_evidence_per_record
            || serde_json::to_vec(&context.facts)
                .map_err(|error| invalid(format!("legal actor facts cannot be encoded: {error}")))?
                .len()
                > self.budgets.max_state_bytes
            || context.context_hash != actor_context_hash(&context)?
        {
            return Err(invalid("legal actor context identity or budget is invalid"));
        }
        let procedure = self
            .procedures
            .get(&requirement.procedure)
            .filter(|procedure| !procedure.closed)
            .ok_or_else(|| invalid("legal actor context targets no open procedure"))?;
        if procedure.active_stage != requirement.stage
            || procedure.round != requirement.round
            || !procedure
                .stages
                .get(procedure.active_stage)
                .is_some_and(|stage| stage.seats.contains(&requirement.seat))
        {
            return Err(invalid(
                "legal actor context targets a stale or ineligible seat",
            ));
        }
        let holder = procedure
            .seat_authorities
            .get(&requirement.seat)
            .map(|authority| &authority.holder)
            .ok_or_else(|| invalid("legal actor context seat authority is missing"))?;
        if holder != &requirement.holder || context.holder != requirement.holder {
            return Err(invalid(
                "legal actor context holder does not match the seat",
            ));
        }
        let key = outbox_key(
            &procedure.id,
            procedure.active_stage,
            procedure.round,
            &requirement.seat,
        );
        if let Some(existing) = self.staged_actor_contexts.get(&key) {
            return if existing == &context {
                Ok(())
            } else {
                Err(invalid("conflicting staged legal actor context"))
            };
        }
        self.reserve_state_growth(Self::encoded_growth(&context, 4)?)?;
        self.staged_actor_contexts.insert(key, context);
        self.dirty_procedures.insert(requirement.procedure.clone());
        Ok(())
    }

    /// Queue a kernel-evaluated holder knowledge query for one pending legal seat.
    pub fn enqueue_actor_context(
        &self,
        plan: &CompiledLawPlan,
        canwu: &mut canwu_api::Canwu,
        requirement: &LegalActorContextRequirement,
        query: &canwu_api::KnowledgeQuery,
    ) -> Result<canwu_api::IngressReceipt, CanwuError> {
        self.ensure_plan_identity(plan)?;
        if query.after.is_some()
            || query.limit == 0
            || usize::try_from(query.limit)
                .ok()
                .is_none_or(|limit| limit > plan.budgets.max_evidence_per_record)
        {
            return Err(invalid(
                "legal actor context requires one bounded, unpaginated knowledge query",
            ));
        }
        if !self
            .pending_actor_context_requirements(plan)?
            .contains(requirement)
        {
            return Err(invalid("legal actor context requirement is stale"));
        }
        canwu.enqueue_plugin_ingress(canwu_api::PluginIngressRequest::new(
            crate::PLUGIN_NAME,
            crate::LAW_ACTOR_CONTEXT_INGRESS,
            canwu.time(),
            serde_json::json!({
                "requirement": requirement,
                "query": query,
            }),
        ))
    }

    /// Derive and stage initial/offline context from the kernel's holder ledger.
    /// Live persisted runtimes should use [`Self::enqueue_actor_context`].
    pub fn stage_actor_context_from_canwu(
        &mut self,
        plan: &CompiledLawPlan,
        requirement: &LegalActorContextRequirement,
        canwu: &canwu_api::Canwu,
        query: &canwu_api::KnowledgeQuery,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        if query.after.is_some()
            || query.limit == 0
            || usize::try_from(query.limit)
                .ok()
                .is_none_or(|limit| limit > plan.budgets.max_evidence_per_record)
        {
            return Err(invalid(
                "legal actor context requires one bounded, unpaginated knowledge query",
            ));
        }
        let result = canwu.admin_query_knowledge(requirement.holder.clone(), query)?;
        if result.next.is_some() || result.holder != requirement.holder {
            return Err(invalid(
                "legal actor context query must fit in one holder-bound page",
            ));
        }
        self.stage_actor_context(requirement, actor_context_from_query_result(&result)?)
    }

    /// Lists active seats that cannot emit an outbox item until the host stages context.
    pub fn pending_actor_context_requirements(
        &self,
        plan: &CompiledLawPlan,
    ) -> Result<Vec<LegalActorContextRequirement>, CanwuError> {
        if self.plan_hash != plan.content_hash {
            return Err(invalid("legal runtime plan hash mismatch"));
        }
        let mut requirements = Vec::new();
        for procedure_id in &self.open_procedures {
            let procedure = self
                .procedures
                .get(procedure_id)
                .ok_or_else(|| invalid("open legal procedure is missing"))?;
            if !self.procedure_has_capacity(plan, procedure)? {
                continue;
            }
            let stage = procedure
                .stages
                .get(procedure.active_stage)
                .ok_or_else(|| invalid("procedure stage index is invalid"))?;
            for seat in &stage.seats {
                let participation =
                    participation_key(&procedure.id, &stage.id, procedure.round, seat);
                let key = outbox_key(&procedure.id, procedure.active_stage, procedure.round, seat);
                if self
                    .latest_participation_by_key
                    .contains_key(&participation)
                    || self.outbox_keys.contains(&key)
                    || self.staged_actor_contexts.contains_key(&key)
                {
                    continue;
                }
                let holder = procedure
                    .seat_authorities
                    .get(seat)
                    .map(|authority| authority.holder.clone())
                    .ok_or_else(|| invalid("legal procedure seat authority is missing"))?;
                requirements.push(LegalActorContextRequirement {
                    procedure: procedure.id.clone(),
                    stage: procedure.active_stage,
                    round: procedure.round,
                    seat: seat.clone(),
                    holder,
                });
            }
        }
        Ok(requirements)
    }

    /// Consumes snapshot-visible intents, advances procedures, adopts laws, applies
    /// due versions, and refreshes sparse applicability results.
    ///
    /// All fallible capacity and reference checks run before mutation. The hot path
    /// visits bounded active indexes and never clones or validates historical state.
    pub fn settle_boundary(
        &mut self,
        plan: &CompiledLawPlan,
        at: SimTime,
        signals: &[LegalSignal],
    ) -> Result<LegalBoundaryResult, CanwuError> {
        let mut procedure_ids = self.procedures_due_for_work(at);
        for signal in signals {
            if let Some(procedure) = self
                .proposals
                .get(&signal.proposal_id)
                .and_then(|proposal| proposal.active_procedure.as_ref())
            {
                procedure_ids.insert(procedure.clone());
            }
        }
        let reserved_growth = self.preflight_boundary(plan, at, signals, &procedure_ids)?;
        self.reserve_state_growth(reserved_growth)?;
        self.boundary_index = self
            .boundary_index
            .checked_add(1)
            .ok_or_else(|| invalid("legal boundary index overflowed"))?;
        self.last_settled_at = at;
        let mut evidence_by_proposal = BTreeMap::<String, BTreeSet<EvidenceRef>>::new();
        let mut kinds_by_proposal = BTreeMap::<String, BTreeSet<String>>::new();
        for signal in signals {
            evidence_by_proposal
                .entry(signal.proposal_id.clone())
                .or_default()
                .extend(signal.evidence.iter().cloned());
            kinds_by_proposal
                .entry(signal.proposal_id.clone())
                .or_default()
                .insert(signal.kind.clone());
        }
        for (proposal_id, evidence) in evidence_by_proposal {
            let added_dependencies = {
                let proposal = self
                    .proposals
                    .get_mut(&proposal_id)
                    .ok_or_else(|| invalid("preflighted legal signal proposal disappeared"))?;
                let added = evidence
                    .iter()
                    .filter(|reference| !proposal.evidence.contains(reference))
                    .cloned()
                    .collect();
                proposal.evidence.extend(evidence);
                proposal.evidence.sort();
                proposal.evidence.dedup();
                proposal
                    .admitted_signal_kinds
                    .extend(kinds_by_proposal.remove(&proposal_id).unwrap_or_default());
                if let Some(procedure) = &proposal.active_procedure {
                    procedure_ids.insert(procedure.clone());
                }
                if proposal.status == ProposalStatus::Submitted {
                    proposal.status = ProposalStatus::Deliberating;
                }
                added
            };
            self.add_identity_evidence_component(added_dependencies)?;
        }
        let mut outcomes = Vec::new();
        let intents = self.pending_intents.values().cloned().collect::<Vec<_>>();
        for intent in intents {
            if let Some(outcome) = self.apply_intent(plan, &intent) {
                outcomes.push(outcome);
            }
        }
        let mut adopted = Vec::new();
        for id in &procedure_ids {
            let deadline = self.procedures.get(id).map(|procedure| procedure.deadline);
            if deadline.is_some_and(|deadline| at > deadline) {
                self.expire_procedure(id, at)?;
                continue;
            }
            if self.advance_procedure(plan, id, at)? {
                if let Some(procedure) = self.procedures.get(id) {
                    adopted.push(procedure.proposal.id.clone());
                }
            }
        }
        let applied = self.apply_scheduled_versions(at)?;
        let emitted = self.materialize_outbox(plan, at, &procedure_ids)?;
        self.prune_staged_actor_contexts(&procedure_ids);
        let refreshed = self.refresh_applicability(at)?;
        for procedure in &procedure_ids {
            self.dirty_procedures.remove(procedure);
        }
        self.intent_outcomes.extend(outcomes.clone());
        Ok(LegalBoundaryResult {
            boundary: self.boundary_index,
            adopted_proposals: adopted,
            rejected_intents: outcomes,
            applied_versions: applied,
            refreshed_effects: refreshed,
            emitted_outbox: emitted,
        })
    }

    fn preflight_boundary(
        &self,
        plan: &CompiledLawPlan,
        at: SimTime,
        signals: &[LegalSignal],
        procedure_ids: &BTreeSet<String>,
    ) -> Result<usize, CanwuError> {
        if self.plan_hash != plan.content_hash {
            return Err(invalid("legal runtime plan hash mismatch"));
        }
        if at < self.last_settled_at {
            return Err(invalid("legal boundaries must be monotonic"));
        }
        self.boundary_index
            .checked_add(1)
            .ok_or_else(|| invalid("legal boundary index overflowed"))?;
        let mut reserved_growth = if signals.is_empty() {
            0
        } else {
            Self::encoded_growth(signals, 4)?
        };

        let mut evidence_counts = BTreeMap::<&str, BTreeSet<EvidenceRef>>::new();
        validate_runtime_text_budget(signals, plan.budgets.max_text_bytes)?;
        for signal in signals {
            if signal.kind.is_empty() || signal.evidence.is_empty() {
                return Err(invalid(
                    "legal signals require a non-empty kind and exact evidence",
                ));
            }
            let proposal = self
                .proposals
                .get(&signal.proposal_id)
                .ok_or_else(|| invalid("legal signal references an unknown proposal"))?;
            let values = evidence_counts
                .entry(signal.proposal_id.as_str())
                .or_insert_with(|| proposal.evidence.iter().cloned().collect());
            values.extend(signal.evidence.iter().cloned());
            if values.len() > plan.budgets.max_evidence_per_record {
                return Err(invalid("legal proposal evidence budget exhausted"));
            }
        }
        let mut possible_adoptions = BTreeSet::new();
        let mut possible_new_rules = BTreeSet::new();
        let mut possible_outbox = 0usize;
        let mut possible_procedure_mutations = 0usize;
        let mut accepted_intents = 0usize;
        let mut rejected_intents = 0usize;
        let mut projected_participations = BTreeSet::<String>::new();
        let mut accepted_by_procedure = BTreeMap::<&str, Vec<&PendingLegalIntent>>::new();
        for intent in self.pending_intents.values() {
            reserved_growth = reserved_growth
                .checked_add(Self::encoded_growth(intent, 8)?)
                .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
            let accepted = self.validate_intent_binding(intent).is_ok()
                && intent.admitted_at <= at
                && self
                    .procedures
                    .get(&intent.procedure.id)
                    .is_some_and(|procedure| {
                        let Some(stage) = procedure.stages.get(procedure.active_stage) else {
                            return false;
                        };
                        let key = participation_key(
                            &procedure.id,
                            &stage.id,
                            procedure.round,
                            &intent.seat,
                        );
                        at <= procedure.deadline
                            && stage.seats.contains(&intent.seat)
                            && (stage.allow_replacement
                                || (!self.latest_participation_by_key.contains_key(&key)
                                    && !projected_participations.contains(&key)))
                    });
            if accepted {
                let procedure = &self.procedures[&intent.procedure.id];
                let stage = &procedure.stages[procedure.active_stage];
                projected_participations.insert(participation_key(
                    &procedure.id,
                    &stage.id,
                    procedure.round,
                    &intent.seat,
                ));
                accepted_intents += 1;
                accepted_by_procedure
                    .entry(intent.procedure.id.as_str())
                    .or_default()
                    .push(intent);
            } else {
                rejected_intents += 1;
            }
        }
        if self
            .participations
            .len()
            .checked_add(accepted_intents)
            .is_none_or(|count| count > plan.budgets.max_participations)
            || self
                .intent_outcomes
                .len()
                .checked_add(rejected_intents)
                .is_none_or(|count| count > plan.budgets.max_intent_outcomes)
        {
            return Err(invalid("legal intent settlement budget exhausted"));
        }
        for procedure_id in procedure_ids {
            let procedure = self
                .procedures
                .get(procedure_id)
                .ok_or_else(|| invalid("open legal procedure is missing"))?;
            let stage = procedure
                .stages
                .get(procedure.active_stage)
                .ok_or_else(|| invalid("procedure stage index is invalid"))?;
            if procedure.active_stage + 1 < procedure.stages.len() {
                at.checked_add(SimDuration::minutes(
                    procedure.stages[procedure.active_stage + 1].deadline_minutes,
                ))
                .ok_or_else(|| invalid("legal procedure deadline overflowed"))?;
            }
            let proposal = self
                .proposals
                .get(&procedure.proposal.id)
                .ok_or_else(|| invalid("procedure proposal is missing"))?;
            let profile = plan
                .source_profile_by_id
                .get(&proposal.source_profile)
                .and_then(|key| plan.source_profiles.get(key.get() as usize))
                .ok_or_else(|| invalid("legal source profile is missing"))?;
            if profile.procedure.as_deref() != Some(proposal.procedure_profile.as_str()) {
                return Err(invalid(
                    "legal source profile does not authorize the proposal procedure",
                ));
            }
            canonical_hash("canwu.law.source.v1", &proposal.clauses)?;
            if !self.procedure_has_capacity(plan, procedure)? {
                continue;
            }
            if let Some(rule) = self.rules.get(&proposal.rule_id) {
                let latest = rule
                    .latest_adopted_version
                    .as_ref()
                    .ok_or_else(|| invalid("legal rule has no latest adopted version"))?;
                self.law_versions
                    .get(&latest.id)
                    .ok_or_else(|| invalid("latest adopted law version is missing"))?
                    .legal_ordinal
                    .checked_add(1)
                    .ok_or_else(|| invalid("legal version ordinal overflowed"))?;
            }
            let mut projected_ballots = BTreeMap::<&str, Ballot>::new();
            for seat in &stage.seats {
                let key = participation_key(&procedure.id, &stage.id, procedure.round, seat);
                if let Some(ballot) = self
                    .latest_participation_by_key
                    .get(&key)
                    .and_then(|index| self.participations.get(*index))
                    .map(|participation| participation.ballot)
                {
                    projected_ballots.insert(seat, ballot);
                }
            }
            for intent in accepted_by_procedure
                .get(procedure.id.as_str())
                .into_iter()
                .flatten()
            {
                if stage.allow_replacement {
                    projected_ballots
                        .insert(&intent.seat, ballot_for_option(&intent.selected_option));
                } else {
                    projected_ballots
                        .entry(&intent.seat)
                        .or_insert_with(|| ballot_for_option(&intent.selected_option));
                }
            }
            let vetoed = projected_ballots
                .values()
                .any(|ballot| *ballot == Ballot::Veto)
                || (stage.kind == ProcedureStageKind::Veto
                    && projected_ballots
                        .values()
                        .any(|ballot| *ballot == Ballot::Against));
            let counted = projected_ballots
                .values()
                .filter(|ballot| **ballot != Ballot::Abstain)
                .count();
            let approved = projected_ballots
                .values()
                .filter(|ballot| **ballot == Ballot::For)
                .count();
            let passed = !vetoed
                && projected_ballots.len() >= stage.quorum as usize
                && counted > 0
                && approved.saturating_mul(1_000)
                    >= counted.saturating_mul(stage.threshold as usize);
            let (projected_stage, projected_round, closes) = if vetoed {
                possible_procedure_mutations += 1;
                reserved_growth = reserved_growth
                    .checked_add(Self::encoded_growth(&(&procedure, proposal), 8)?)
                    .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
                (procedure.active_stage, procedure.round, true)
            } else if passed && procedure.active_stage + 1 == procedure.stages.len() {
                self.validate_adoption_guards(plan, proposal, at)?;
                possible_procedure_mutations += 1;
                reserved_growth = reserved_growth
                    .checked_add(Self::encoded_growth(&(&procedure, proposal), 24)?)
                    .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
                if self.validate_proposal_rule_transition(proposal).is_ok() {
                    possible_adoptions.insert(procedure.id.as_str());
                    if !self.rules.contains_key(&proposal.rule_id) {
                        possible_new_rules.insert(proposal.rule_id.as_str());
                    }
                }
                (procedure.active_stage, procedure.round, true)
            } else if passed {
                possible_procedure_mutations += 1;
                reserved_growth = reserved_growth
                    .checked_add(Self::encoded_growth(procedure, 8)?)
                    .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
                (
                    procedure.active_stage + 1,
                    procedure.round.saturating_add(1),
                    false,
                )
            } else if at > procedure.deadline {
                possible_procedure_mutations += 1;
                reserved_growth = reserved_growth
                    .checked_add(Self::encoded_growth(&(&procedure, proposal), 8)?)
                    .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
                (procedure.active_stage, procedure.round, true)
            } else {
                (procedure.active_stage, procedure.round, false)
            };
            if !closes && at < procedure.deadline {
                let projected = &procedure.stages[projected_stage];
                let missing = projected
                    .seats
                    .iter()
                    .filter(|seat| {
                        let has_projected_ballot = projected_stage == procedure.active_stage
                            && projected_ballots.contains_key(seat.as_str());
                        let key = outbox_key(&procedure.id, projected_stage, projected_round, seat);
                        !has_projected_ballot
                            && !self.outbox_keys.contains(&key)
                            && self.staged_actor_contexts.contains_key(&key)
                    })
                    .count();
                possible_outbox = possible_outbox
                    .checked_add(missing)
                    .ok_or_else(|| invalid("legal outbox preflight overflowed"))?;
                if missing > 0 {
                    reserved_growth = reserved_growth
                        .checked_add(
                            Self::encoded_growth(&(&procedure, proposal), 12)?
                                .checked_mul(missing)
                                .ok_or_else(|| {
                                    invalid("legal serialized-state accounting overflowed")
                                })?,
                        )
                        .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
                }
            }
        }
        if self
            .sources
            .len()
            .checked_add(possible_adoptions.len())
            .is_none_or(|count| count > plan.budgets.max_sources)
            || self
                .law_versions
                .len()
                .checked_add(possible_adoptions.len())
                .is_none_or(|count| count > plan.budgets.max_law_versions)
            || self
                .rules
                .len()
                .checked_add(possible_new_rules.len())
                .is_none_or(|count| count > plan.budgets.max_rules)
        {
            return Err(invalid("legal adoption budget exhausted"));
        }
        self.next_source_ordinal
            .checked_add(possible_adoptions.len() as u64)
            .ok_or_else(|| invalid("legal source ordinal overflowed"))?;
        if self
            .outbox
            .len()
            .checked_add(possible_outbox)
            .is_none_or(|count| count > plan.budgets.max_outbox)
        {
            return Err(invalid("legal decision outbox budget exhausted"));
        }
        if possible_outbox > 0 {
            let final_sequence = self
                .next_outbox_sequence
                .checked_add(possible_outbox as u64 - 1)
                .ok_or_else(|| invalid("legal outbox sequence overflowed"))?;
            allocate_outbox_ids(plan, final_sequence)?;
        }
        let due_count = self
            .scheduled_versions_by_time
            .range(..=at)
            .try_fold(0usize, |count, (_, references)| {
                count.checked_add(references.len())
            })
            .ok_or_else(|| invalid("legal due-version count overflowed"))?;
        let mut projected_applicability_entries = 0usize;
        let mut applicability_deletion_rules = BTreeSet::new();
        for procedure_id in &possible_adoptions {
            let proposal = &self.proposals[&self.procedures[*procedure_id].proposal.id];
            applicability_deletion_rules.insert(proposal.rule_id.clone());
            projected_applicability_entries = projected_applicability_entries
                .checked_add(
                    proposal
                        .clauses
                        .len()
                        .checked_mul(proposal.jurisdictions.len())
                        .ok_or_else(|| invalid("legal applicability projection overflowed"))?,
                )
                .ok_or_else(|| invalid("legal applicability projection overflowed"))?;
        }
        for (effective_at, references) in self.scheduled_versions_by_time.range(..=at) {
            for reference in references {
                let version = self
                    .law_versions
                    .get(&reference.id)
                    .ok_or_else(|| invalid("scheduled law version is missing"))?;
                let rule = self
                    .rules
                    .get(&version.rule)
                    .ok_or_else(|| invalid("scheduled law version has no rule"))?;
                applicability_deletion_rules.insert(version.rule.clone());
                if version.effective_at != *effective_at
                    || rule.scheduled_versions.binary_search(reference).is_err()
                {
                    return Err(invalid("legal effective-time index is inconsistent"));
                }
                reserved_growth = reserved_growth
                    .checked_add(Self::encoded_growth(version, 8)?)
                    .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
                reserved_growth = reserved_growth
                    .checked_add(
                        version
                            .deltas
                            .len()
                            .checked_mul(version.jurisdictions.len())
                            .and_then(|count| count.checked_mul(2_048))
                            .ok_or_else(|| {
                                invalid("legal serialized-state accounting overflowed")
                            })?,
                    )
                    .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
                projected_applicability_entries = projected_applicability_entries
                    .checked_add(
                        version
                            .deltas
                            .len()
                            .checked_mul(version.jurisdictions.len())
                            .ok_or_else(|| invalid("legal applicability projection overflowed"))?,
                    )
                    .ok_or_else(|| invalid("legal applicability projection overflowed"))?;
            }
        }
        for rule_id in &self.dirty_rules {
            applicability_deletion_rules.insert(rule_id.clone());
            let rule = self
                .rules
                .get(rule_id)
                .ok_or_else(|| invalid("dirty legal rule is missing"))?;
            reserved_growth = reserved_growth
                .checked_add(Self::encoded_growth(rule, 8)?)
                .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
            if let Some(reference) = &rule.operative_version {
                let version = self
                    .law_versions
                    .get(&reference.id)
                    .ok_or_else(|| invalid("operative law version is missing"))?;
                let projection_count = version
                    .deltas
                    .len()
                    .checked_mul(version.jurisdictions.len())
                    .ok_or_else(|| invalid("legal applicability projection overflowed"))?;
                projected_applicability_entries = projected_applicability_entries
                    .checked_add(projection_count)
                    .ok_or_else(|| invalid("legal applicability projection overflowed"))?;
                reserved_growth =
                    reserved_growth
                        .checked_add(projection_count.checked_mul(2_048).ok_or_else(|| {
                            invalid("legal serialized-state accounting overflowed")
                        })?)
                        .ok_or_else(|| invalid("legal serialized-state accounting overflowed"))?;
            }
        }
        let projected_applicability_deletions = applicability_deletion_rules
            .iter()
            .try_fold(0usize, |count, rule_id| {
                count.checked_add(
                    self.applicability_by_rule
                        .get(rule_id)
                        .map_or(0, BTreeSet::len),
                )
            })
            .ok_or_else(|| invalid("legal applicability deletion budget overflowed"))?;
        let projected_applicability_operations = projected_applicability_entries
            .checked_add(projected_applicability_deletions)
            .ok_or_else(|| invalid("legal applicability work budget overflowed"))?;
        if projected_applicability_operations > plan.budgets.max_applicability_entries_per_boundary
        {
            return Err(invalid("legal applicability projection budget exhausted"));
        }
        let mutations = evidence_counts
            .len()
            .checked_add(self.pending_intents.len())
            .and_then(|count| count.checked_add(possible_procedure_mutations))
            .and_then(|count| count.checked_add(due_count))
            .and_then(|count| count.checked_add(projected_applicability_operations))
            .and_then(|count| count.checked_add(possible_outbox))
            .ok_or_else(|| invalid("legal boundary mutation count overflowed"))?;
        if mutations > plan.budgets.max_mutations_per_boundary {
            return Err(invalid("legal boundary mutation budget exhausted"));
        }
        self.ensure_state_growth(reserved_growth)?;
        Ok(reserved_growth)
    }

    fn apply_intent(
        &mut self,
        plan: &CompiledLawPlan,
        intent: &PendingLegalIntent,
    ) -> Option<LegalIntentOutcome> {
        if let Err(error) = self.validate_intent_binding(intent) {
            let reason = error.message;
            return Some(self.reject_intent(intent, &reason));
        }
        let Some(procedure) = self.procedures.get(&intent.procedure.id).cloned() else {
            return Some(self.reject_intent(intent, "unknown procedure"));
        };
        if !self.proposals.contains_key(&intent.proposal.id) {
            return Some(self.reject_intent(intent, "unknown proposal"));
        }
        let valid_seat = procedure
            .stages
            .get(procedure.active_stage)
            .is_some_and(|stage| stage.seats.contains(&intent.seat));
        let stale = intent.round != procedure.round || intent.stage != procedure.active_stage;
        if !valid_seat || stale || self.last_settled_at > procedure.deadline {
            return Some(self.reject_intent(
                intent,
                if !valid_seat {
                    "seat is not eligible"
                } else if stale {
                    "procedure round or stage is stale"
                } else {
                    "procedure deadline expired"
                },
            ));
        }
        if intent.admitted_at > self.last_settled_at {
            return Some(self.reject_intent(intent, "intent was admitted after this boundary"));
        }
        let stage_id = &procedure.stages[procedure.active_stage].id;
        let key = participation_key(&procedure.id, stage_id, procedure.round, &intent.seat);
        let replaced = self
            .latest_participation_by_key
            .get(&key)
            .and_then(|index| self.participations.get(*index));
        if replaced.is_some() && !procedure.stages[procedure.active_stage].allow_replacement {
            return Some(self.reject_intent(intent, "seat already participated"));
        }
        let id = format!("participation:{}", intent.id);
        if self.participations.len() >= plan.budgets.max_participations {
            return Some(self.reject_intent(intent, "procedure participation budget exhausted"));
        }
        self.participations.push(ProcedureParticipation {
            id,
            procedure: procedure_reference(&procedure),
            stage: stage_id.clone(),
            round: procedure.round,
            seat: intent.seat.clone(),
            controller: intent.controller.clone(),
            ballot: ballot_for_option(&intent.selected_option),
            option_id: intent.selected_option.clone(),
            admitted_at: self.last_settled_at,
            command: Some(intent.command.clone()),
            replaced: replaced.map(|participation| local_ref("participation", &participation.id)),
        });
        self.latest_participation_by_key
            .insert(key, self.participations.len() - 1);
        self.acknowledge_intent_outbox(intent);
        self.pending_intents.remove(&intent.id);
        self.consumed_intent_ids.insert(intent.id.clone());
        let _ = plan;
        None
    }

    fn reject_intent(&mut self, intent: &PendingLegalIntent, reason: &str) -> LegalIntentOutcome {
        self.acknowledge_intent_outbox(intent);
        self.pending_intents.remove(&intent.id);
        self.consumed_intent_ids.insert(intent.id.clone());
        LegalIntentOutcome {
            intent: intent.id.clone(),
            status: if reason.contains("deadline") {
                LegalIntentStatus::Expired
            } else {
                LegalIntentStatus::Rejected
            },
            reason: Some(reason.to_owned()),
            source: None,
            law_versions: Vec::new(),
            at: self.last_settled_at,
        }
    }

    fn acknowledge_intent_outbox(&mut self, intent: &PendingLegalIntent) {
        let key = outbox_key(
            &intent.procedure.id,
            intent.stage,
            intent.round,
            &intent.seat,
        );
        let Some(sequence) = self.outbox_sequence_by_key.get(&key).copied() else {
            return;
        };
        let Some(item) = self.outbox.get_mut(&sequence) else {
            return;
        };
        if item.dispatch == DispatchState::Enqueued {
            item.dispatch = DispatchState::Acknowledged;
            item.acknowledgement = Some(
                intent
                    .attempt
                    .clone()
                    .unwrap_or_else(|| intent.command.clone()),
            );
        }
    }

    fn procedure_has_capacity(
        &self,
        plan: &CompiledLawPlan,
        procedure: &ProcedureInstance,
    ) -> Result<bool, CanwuError> {
        let profile = plan
            .procedure_by_id
            .get(&procedure.profile)
            .and_then(|key| plan.procedures.get(key.get() as usize))
            .ok_or_else(|| invalid("legal procedure profile is missing"))?;
        Ok(profile.reservation_pool.as_ref().is_none_or(|pool| {
            self.capacity_allocations
                .get(&procedure.id)
                .is_some_and(|allocation| {
                    &allocation.pool == pool && allocation.quantity == profile.reservation_quantity
                })
        }))
    }

    fn procedures_due_for_work(&self, at: SimTime) -> BTreeSet<String> {
        let mut procedures = self.dirty_procedures.clone();
        for due in self.procedures_by_deadline.range(..=at).map(|(_, ids)| ids) {
            procedures.extend(due.iter().cloned());
        }
        procedures
    }

    pub(crate) fn next_due_time(&self) -> Option<SimTime> {
        self.procedures_by_deadline
            .keys()
            .next()
            .copied()
            .into_iter()
            .chain(self.scheduled_versions_by_time.keys().next().copied())
            .min()
    }

    pub(crate) fn mark_wake_scheduled(&mut self, at: SimTime) -> Result<bool, CanwuError> {
        if self.scheduled_wakes.contains(&at) {
            return Ok(false);
        }
        self.reserve_state_growth(Self::encoded_growth(&at, 2)?)?;
        Ok(self.scheduled_wakes.insert(at))
    }

    pub(crate) fn consume_wake(&mut self, at: SimTime) -> Result<(), CanwuError> {
        if !self.scheduled_wakes.remove(&at) {
            return Err(invalid("legal wake ingress is stale or untracked"));
        }
        Ok(())
    }

    fn move_procedure_deadline(
        &mut self,
        id: &str,
        previous: SimTime,
        next: SimTime,
    ) -> Result<(), CanwuError> {
        let previous = procedure_expiry_time(previous)?;
        let next = procedure_expiry_time(next)?;
        let indexed = self
            .procedures_by_deadline
            .get_mut(&previous)
            .ok_or_else(|| invalid("legal procedure deadline index is missing"))?;
        if !indexed.remove(id) {
            return Err(invalid("legal procedure deadline identity is missing"));
        }
        if indexed.is_empty() {
            self.procedures_by_deadline.remove(&previous);
        }
        self.procedures_by_deadline
            .entry(next)
            .or_default()
            .insert(id.to_owned());
        Ok(())
    }

    fn close_procedure(&mut self, id: &str) -> Result<(), CanwuError> {
        let procedure = self
            .procedures
            .get(id)
            .ok_or_else(|| invalid("legal procedure is missing"))?;
        let deadline = procedure.deadline;
        let procedure_dependencies = procedure.evidence.iter().cloned().collect();
        let proposal_dependencies = self
            .proposals
            .get(&procedure.proposal.id)
            .filter(|proposal| {
                !matches!(
                    proposal.status,
                    ProposalStatus::Draft
                        | ProposalStatus::Submitted
                        | ProposalStatus::Deliberating
                )
            })
            .map(Self::proposal_identity_evidence_dependencies);
        let expiry = procedure_expiry_time(deadline)?;
        if let Some(current) = self.procedures.get_mut(id) {
            current.closed = true;
        }
        self.open_procedures.remove(id);
        self.dirty_procedures.remove(id);
        let indexed = self
            .procedures_by_deadline
            .get_mut(&expiry)
            .ok_or_else(|| invalid("legal procedure deadline index is missing"))?;
        if !indexed.remove(id) {
            return Err(invalid("legal procedure deadline identity is missing"));
        }
        if indexed.is_empty() {
            self.procedures_by_deadline.remove(&expiry);
        }
        self.remove_identity_evidence_component(procedure_dependencies)?;
        if let Some(dependencies) = proposal_dependencies {
            self.remove_identity_evidence_component(dependencies)?;
        }
        Ok(())
    }

    fn expire_procedure(&mut self, id: &str, _at: SimTime) -> Result<(), CanwuError> {
        let proposal_id = self
            .procedures
            .get(id)
            .ok_or_else(|| invalid("expiring legal procedure is missing"))?
            .proposal
            .id
            .clone();
        if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
            proposal.status = ProposalStatus::Expired;
        }
        let sequences = self
            .outbox
            .iter()
            .filter(|(_, item)| {
                item.procedure.id == id
                    && matches!(
                        item.dispatch,
                        DispatchState::Pending | DispatchState::Enqueued
                    )
            })
            .map(|(sequence, _)| *sequence)
            .collect::<Vec<_>>();
        for sequence in sequences {
            if let Some(item) = self.outbox.get_mut(&sequence) {
                item.dispatch = DispatchState::Expired;
            }
            self.pending_outbox_sequences.remove(&sequence);
        }
        self.close_procedure(id)
    }

    fn advance_procedure(
        &mut self,
        plan: &CompiledLawPlan,
        id: &str,
        at: SimTime,
    ) -> Result<bool, CanwuError> {
        let Some(procedure) = self.procedures.get(id).cloned() else {
            return Ok(false);
        };
        if procedure.closed {
            return Ok(false);
        }
        if !self.procedure_has_capacity(plan, &procedure)? {
            return Ok(false);
        }
        let stage = procedure
            .stages
            .get(procedure.active_stage)
            .ok_or_else(|| invalid("procedure stage index is invalid"))?;
        let votes = stage
            .seats
            .iter()
            .filter_map(|seat| {
                let key = participation_key(&procedure.id, &stage.id, procedure.round, seat);
                self.latest_participation_by_key
                    .get(&key)
                    .and_then(|index| self.participations.get(*index))
            })
            .collect::<Vec<_>>();
        if votes.iter().any(|vote| vote.ballot == Ballot::Veto)
            || (stage.kind == ProcedureStageKind::Veto
                && votes.iter().any(|vote| vote.ballot == Ballot::Against))
        {
            let proposal_id = procedure.proposal.id.clone();
            if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
                proposal.status = ProposalStatus::Rejected;
            }
            self.close_procedure(id)?;
            return Ok(false);
        }
        if votes.len() < stage.quorum as usize {
            return Ok(false);
        }
        let counted = votes
            .iter()
            .filter(|vote| vote.ballot != Ballot::Abstain)
            .count();
        let approved = votes
            .iter()
            .filter(|vote| vote.ballot == Ballot::For)
            .count();
        if counted == 0
            || approved.saturating_mul(1_000) < counted.saturating_mul(stage.threshold as usize)
        {
            return Ok(false);
        }
        if procedure.active_stage + 1 < procedure.stages.len() {
            let next_stage = &procedure.stages[procedure.active_stage + 1];
            let deadline = at
                .checked_add(SimDuration::minutes(next_stage.deadline_minutes))
                .ok_or_else(|| invalid("legal procedure deadline overflowed"))?;
            if let Some(current) = self.procedures.get_mut(id) {
                current.active_stage += 1;
                current.round = current.round.saturating_add(1);
                current.eligible_seats.clone_from(&next_stage.seats);
                current.deadline = deadline;
            }
            self.move_procedure_deadline(id, procedure.deadline, deadline)?;
            return Ok(false);
        }
        let proposal = self
            .proposals
            .get(&procedure.proposal.id)
            .ok_or_else(|| invalid("procedure proposal is missing"))?;
        let source_profile = plan
            .source_profile_by_id
            .get(&proposal.source_profile)
            .and_then(|key| plan.source_profiles.get(key.get() as usize))
            .ok_or_else(|| invalid("legal source profile is missing"))?;
        if source_profile
            .required_signal_kinds
            .iter()
            .any(|kind| !proposal.admitted_signal_kinds.contains(kind))
        {
            return Ok(false);
        }
        if self.validate_proposal_rule_transition(proposal).is_err() {
            if let Some(proposal) = self.proposals.get_mut(&procedure.proposal.id) {
                proposal.status = ProposalStatus::Rejected;
            }
            self.close_procedure(id)?;
            return Ok(false);
        }
        self.adopt_proposal(plan, &procedure, at)?;
        Ok(true)
    }

    fn adopt_proposal(
        &mut self,
        plan: &CompiledLawPlan,
        procedure: &ProcedureInstance,
        at: SimTime,
    ) -> Result<(), CanwuError> {
        let proposal_id = procedure.proposal.id.clone();
        self.adopt_proposal_record(plan, &proposal_id, Some(procedure), at)
    }

    fn adopt_proposal_record(
        &mut self,
        plan: &CompiledLawPlan,
        proposal_id: &str,
        procedure: Option<&ProcedureInstance>,
        at: SimTime,
    ) -> Result<(), CanwuError> {
        let proposal_snapshot = self
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or_else(|| invalid("legal source proposal is missing"))?;
        self.validate_adoption_guards(plan, &proposal_snapshot, at)?;
        if proposal_snapshot
            .cultural_dependencies
            .iter()
            .any(|dependency| self.retired_cultural_targets.contains(&dependency.target))
        {
            return Err(invalid(
                "legal adoption depends on a retired culture generation",
            ));
        }
        self.validate_proposal_rule_transition(&proposal_snapshot)?;
        let proposal_dependencies =
            Self::proposal_identity_evidence_dependencies(&proposal_snapshot);
        let replaced_operative_dependencies = (proposal_snapshot.effective_at <= at
            && proposal_snapshot.validity == OperativeDisposition::Operative)
            .then(|| self.rules.get(&proposal_snapshot.rule_id))
            .flatten()
            .filter(|rule| !rule.retired)
            .and_then(|rule| rule.operative_version.as_ref())
            .map(|reference| self.law_version_identity_evidence_dependencies(reference));
        let replaced_latest_claim_dependencies = self
            .rules
            .get(&proposal_snapshot.rule_id)
            .and_then(|rule| rule.latest_adopted_version.as_ref())
            .filter(|reference| {
                self.law_versions
                    .get(&reference.id)
                    .is_some_and(|version| !materializes_legal_effect(version))
            })
            .map(|reference| self.law_version_identity_evidence_dependencies(reference));
        if let Some(proposal) = self.proposals.get_mut(proposal_id) {
            proposal.status = ProposalStatus::Adopted;
        }
        let proposal = &proposal_snapshot;
        let profile = plan
            .source_profile_by_id
            .get(&proposal.source_profile)
            .and_then(|key| plan.source_profiles.get(key.get() as usize))
            .ok_or_else(|| invalid("legal source profile is missing"))?;
        let publicity_event = proposal
            .publicity
            .as_ref()
            .map(|reference| {
                self.publicity_events
                    .get(&reference.id)
                    .filter(|event| {
                        reference == &local_ref("publicity", &event.id)
                            && event.proposal == local_ref("proposal", &proposal.id)
                    })
                    .ok_or_else(|| invalid("legal proposal publicity event is not exact"))
            })
            .transpose()?;
        let authorized = match (profile.procedure.as_deref(), procedure) {
            (Some(expected), Some(procedure)) => {
                expected == proposal.procedure_profile && procedure.profile == expected
            }
            (None, None) => proposal.procedure_profile.is_empty(),
            _ => false,
        };
        if !authorized {
            return Err(invalid(
                "legal source profile does not authorize the proposal procedure",
            ));
        }
        let source_ordinal = self.next_source_ordinal;
        let source_id = format!("source:{}:{}", proposal.id, self.next_source_ordinal);
        self.next_source_ordinal = self
            .next_source_ordinal
            .checked_add(1)
            .ok_or_else(|| invalid("legal source ordinal overflowed"))?;
        let content_hash = canonical_hash("canwu.law.source.v1", &proposal.clauses)?;
        let publicity_satisfied = !matches!(
            profile.publicity_policy,
            PublicityPolicy::ValidityCondition | PublicityPolicy::EffectivenessCondition
        ) || publicity_event.is_some();
        let source = LegalSourceVersion {
            id: source_id.clone(),
            ordinal: source_ordinal,
            proposal: local_ref("proposal", &proposal.id),
            mode: profile.mode,
            legal_order: proposal.legal_order.clone(),
            applicability_profile: profile.applicability_profile.clone(),
            issuer: proposal.sponsor.clone(),
            claimant: (profile.procedure.is_none())
                .then(|| proposal.sponsor.clone().map(KnowledgeHolderRef::Entity))
                .flatten(),
            procedure: procedure.map(procedure_reference),
            content_hash: content_hash.clone(),
            text_hash: content_hash.clone(),
            competence_claim: if proposal.procedure_profile.is_empty() {
                proposal.source_profile.clone()
            } else {
                proposal.procedure_profile.clone()
            },
            competence: proposal.competence,
            validity: proposal.validity,
            origin: proposal.origin.clone(),
            authority_policy: profile.authority_policy,
            publicity_policy: profile.publicity_policy,
            publicity_event: proposal.publicity.clone(),
            publicity: publicity_event.map_or_else(
                || {
                    if profile.publicity_policy == PublicityPolicy::EffectivenessCondition {
                        "not_yet_published".to_owned()
                    } else {
                        "not_required".to_owned()
                    }
                },
                |event| event.medium.clone(),
            ),
            defects: proposal.defects.clone(),
            evidence_kinds: proposal.admitted_signal_kinds.iter().cloned().collect(),
            adopted_at: at,
            promulgated_at: publicity_event.map(|event| event.at),
            effective_at: proposal.effective_at,
            expires_at: None,
            evidence: proposal.evidence.clone(),
            cultural_dependencies: proposal.cultural_dependencies.clone(),
        };
        self.sources.insert(source_id.clone(), source);
        let rule_id = proposal.rule_id.clone();
        let ordinal = self.rules.get(&rule_id).map_or(Ok(1), |rule| {
            let latest = rule
                .latest_adopted_version
                .as_ref()
                .ok_or_else(|| invalid("legal rule has no latest adopted version"))?;
            self.law_versions
                .get(&latest.id)
                .ok_or_else(|| invalid("latest adopted law version is missing"))?
                .legal_ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("legal version ordinal overflowed"))
        })?;
        let version_id = format!("law-version:{}:{}", rule_id, ordinal);
        let source_ref = local_ref("source_version", &source_id);
        let effects = proposal
            .clauses
            .iter()
            .enumerate()
            .map(|(index, clause)| {
                let modality = plan
                    .clauses
                    .iter()
                    .find(|candidate| candidate.id == clause.clause)
                    .map(|candidate| candidate.modality)
                    .ok_or_else(|| invalid("legal clause lacks a compiled modality"))?;
                Ok(NormativeEffect {
                    id: format!("effect:{}:{}", proposal.id, index),
                    modality,
                    holders: if clause.holders.is_empty() {
                        proposal.subjects.iter().map(ToString::to_string).collect()
                    } else {
                        clause.holders.clone()
                    },
                    duty_bearers: clause.duty_bearers.clone(),
                    subject_matters: clause.subject_matters.clone(),
                    territories: clause.territories.clone(),
                    action: clause.operation.clone(),
                    object: clause.value.to_string(),
                    conditions: clause.conditions.clone(),
                    exceptions: clause.exceptions.clone(),
                    standing: clause.standing.clone(),
                    forum: clause.forum.clone(),
                    remedy_profile: clause.remedy_profile.clone(),
                    source_refs: vec![source_ref.clone()],
                })
            })
            .collect::<Result<Vec<_>, CanwuError>>()?;
        let mut predecessors = self
            .rules
            .get(&rule_id)
            .and_then(|rule| rule.latest_adopted_version.clone())
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(LegalOriginRef::Reception { predecessor, .. }) = &proposal.origin {
            predecessors.push(predecessor.clone());
            predecessors.sort();
            predecessors.dedup();
        }
        let version = LawVersion {
            id: version_id.clone(),
            rule: rule_id.clone(),
            legal_ordinal: ordinal,
            operation: proposal.operation,
            applicability_profile: profile.applicability_profile.clone(),
            source: source_ref,
            origin: proposal.origin.clone(),
            predecessors,
            deltas: effects.clone(),
            jurisdictions: proposal.jurisdictions.clone(),
            adopted_at: at,
            promulgated_at: publicity_event.map(|event| event.at),
            effective_at: proposal.effective_at,
            retrospective_from: proposal.retrospective_from,
            disposition: legal_version_disposition(proposal),
            evidence: proposal.evidence.clone(),
            cultural_dependencies: proposal.cultural_dependencies.clone(),
        };
        self.law_versions.insert(version_id.clone(), version);
        let version_ref = local_ref("law_version", &version_id);
        if let Some(proposal) = self.proposals.get_mut(proposal_id) {
            proposal.adopted_at = Some(at);
            proposal.source_version = Some(local_ref("source_version", &source_id));
            proposal.law_version = Some(version_ref.clone());
        }
        let materializes =
            proposal.validity == OperativeDisposition::Operative && publicity_satisfied;
        self.rules
            .entry(rule_id.clone())
            .and_modify(|rule| {
                rule.latest_adopted_version = Some(version_ref.clone());
                if materializes && proposal.effective_at > at {
                    rule.scheduled_versions.push(version_ref.clone());
                    rule.scheduled_versions.sort();
                    rule.scheduled_versions.dedup();
                } else if materializes {
                    rule.operative_version = Some(version_ref.clone());
                    rule.effects.clone_from(&effects);
                    rule.retired = retires_rule(proposal.operation);
                }
            })
            .or_insert(LegalRule {
                id: rule_id.clone(),
                legal_order: proposal.legal_order.clone(),
                latest_adopted_version: Some(version_ref.clone()),
                operative_version: (materializes && proposal.effective_at <= at)
                    .then_some(version_ref.clone()),
                scheduled_versions: if materializes && proposal.effective_at > at {
                    vec![version_ref.clone()]
                } else {
                    Vec::new()
                },
                effects: if materializes && proposal.effective_at <= at {
                    effects
                } else {
                    Vec::new()
                },
                retired: materializes
                    && proposal.effective_at <= at
                    && retires_rule(proposal.operation),
            });
        self.rule_ids_by_order
            .entry(proposal.legal_order.clone())
            .or_default()
            .insert(rule_id.clone());
        let versions = self
            .law_versions_by_rule
            .entry(rule_id.clone())
            .or_default();
        versions.push(version_ref.clone());
        versions.sort_by(|left, right| {
            let left = &self.law_versions[&left.id];
            let right = &self.law_versions[&right.id];
            (left.effective_at, left.legal_ordinal, &left.id).cmp(&(
                right.effective_at,
                right.legal_ordinal,
                &right.id,
            ))
        });
        self.dirty_rules.insert(proposal.rule_id.clone());
        if materializes && proposal.effective_at > at {
            let scheduled = self
                .scheduled_versions_by_time
                .entry(proposal.effective_at)
                .or_default();
            scheduled.push(version_ref.clone());
            scheduled.sort();
            scheduled.dedup();
            for dependency in &proposal.cultural_dependencies {
                if dependency.kind == CulturalDependencyKind::LiveLevel {
                    self.scheduled_live_dependencies
                        .entry(dependency.target.clone())
                        .or_default()
                        .insert(version_ref.clone());
                }
            }
        }
        if let Some(procedure) = procedure {
            self.close_procedure(&procedure.id)?;
        } else {
            self.remove_identity_evidence_component(proposal_dependencies)?;
        }
        if let Some(dependencies) = replaced_operative_dependencies {
            self.remove_identity_evidence_component(dependencies)?;
        }
        if let Some(dependencies) = replaced_latest_claim_dependencies {
            self.remove_identity_evidence_component(dependencies)?;
        }
        if !materializes || proposal.effective_at > at || !retires_rule(proposal.operation) {
            let dependencies = self.law_version_identity_evidence_dependencies(&version_ref);
            self.add_identity_evidence_component(dependencies)?;
        }
        Ok(())
    }

    fn apply_scheduled_versions(&mut self, at: SimTime) -> Result<Vec<String>, CanwuError> {
        let mut applied = Vec::new();
        let due_times = self
            .scheduled_versions_by_time
            .range(..=at)
            .map(|(effective_at, _)| *effective_at)
            .collect::<Vec<_>>();
        for effective_at in due_times {
            let mut references = self
                .scheduled_versions_by_time
                .remove(&effective_at)
                .expect("due effective-time key was indexed");
            references.sort_by(|left, right| {
                let left = &self.law_versions[&left.id];
                let right = &self.law_versions[&right.id];
                (left.legal_ordinal, &left.id).cmp(&(right.legal_ordinal, &right.id))
            });
            for reference in references {
                let version = self
                    .law_versions
                    .get(&reference.id)
                    .cloned()
                    .ok_or_else(|| invalid("scheduled law version is missing"))?;
                if version.effective_at != effective_at {
                    return Err(invalid(
                        "scheduled law version has the wrong effective time",
                    ));
                }
                if version.cultural_dependencies.iter().any(|dependency| {
                    dependency.kind == CulturalDependencyKind::LiveLevel
                        && self.retired_cultural_targets.contains(&dependency.target)
                }) {
                    return Err(invalid(
                        "scheduled law version depends on a retired culture generation",
                    ));
                }
                for dependency in &version.cultural_dependencies {
                    if dependency.kind == CulturalDependencyKind::LiveLevel
                        && let Some(indexed) =
                            self.scheduled_live_dependencies.get_mut(&dependency.target)
                    {
                        indexed.remove(&reference);
                        if indexed.is_empty() {
                            self.scheduled_live_dependencies.remove(&dependency.target);
                        }
                    }
                }
                let replaced_operative_dependencies = self
                    .rules
                    .get(&version.rule)
                    .filter(|rule| !rule.retired)
                    .and_then(|rule| rule.operative_version.as_ref())
                    .map(|operative| self.law_version_identity_evidence_dependencies(operative));
                let rule = self
                    .rules
                    .get_mut(&version.rule)
                    .ok_or_else(|| invalid("scheduled law version has no rule"))?;
                rule.operative_version = Some(reference.clone());
                rule.effects.clone_from(&version.deltas);
                rule.retired = retires_rule(version.operation);
                let position = rule
                    .scheduled_versions
                    .binary_search(&reference)
                    .map_err(|_| invalid("rule effective-time index is inconsistent"))?;
                rule.scheduled_versions.remove(position);
                self.dirty_rules.insert(version.rule.clone());
                if let Some(dependencies) = replaced_operative_dependencies {
                    self.remove_identity_evidence_component(dependencies)?;
                }
                if retires_rule(version.operation) {
                    let dependencies = self.law_version_identity_evidence_dependencies(&reference);
                    self.remove_identity_evidence_component(dependencies)?;
                }
                applied.push(version.id.clone());
            }
        }
        Ok(applied)
    }

    fn materialize_outbox(
        &mut self,
        plan: &CompiledLawPlan,
        at: SimTime,
        procedure_ids: &BTreeSet<String>,
    ) -> Result<Vec<LegalDecisionOutboxItem>, CanwuError> {
        let mut emitted = Vec::new();
        for procedure_id in procedure_ids {
            let procedure = self
                .procedures
                .get(procedure_id)
                .cloned()
                .ok_or_else(|| invalid("open legal procedure is missing"))?;
            if procedure.closed || at > procedure.deadline {
                continue;
            }
            if !self.procedure_has_capacity(plan, &procedure)? {
                continue;
            }
            let proposal = self
                .proposals
                .get(&procedure.proposal.id)
                .cloned()
                .ok_or_else(|| invalid("open legal procedure has no proposal"))?;
            let Some(stage) = procedure.stages.get(procedure.active_stage) else {
                continue;
            };
            for seat in &stage.seats {
                let participation =
                    participation_key(&procedure.id, &stage.id, procedure.round, seat);
                let outbox =
                    outbox_key(&procedure.id, procedure.active_stage, procedure.round, seat);
                if self
                    .latest_participation_by_key
                    .contains_key(&participation)
                    || self.outbox_keys.contains(&outbox)
                {
                    continue;
                }
                let staged_key =
                    outbox_key(&procedure.id, procedure.active_stage, procedure.round, seat);
                let Some(actor_context) = self.staged_actor_contexts.remove(&staged_key) else {
                    continue;
                };
                if self.outbox.len() >= plan.budgets.max_outbox {
                    return Err(invalid("legal decision outbox budget exhausted"));
                }
                let sequence = self.next_outbox_sequence;
                self.next_outbox_sequence = self
                    .next_outbox_sequence
                    .checked_add(1)
                    .ok_or_else(|| invalid("legal outbox sequence overflowed"))?;
                let authority = procedure
                    .seat_authorities
                    .get(seat)
                    .ok_or_else(|| invalid("legal procedure seat authority is missing"))?;
                let controller = authority.holder.clone();
                let permission_profile_id = authority.permission_profile_id.clone();
                let decision_controller_id = authority.decision_controller_id.clone();
                let ids = allocate_outbox_ids(plan, sequence)?;
                if actor_context.holder != controller
                    || actor_context.context_hash != actor_context_hash(&actor_context)?
                {
                    return Err(invalid("staged legal actor context is invalid"));
                }
                let draft = decision_ticket_draft(
                    &proposal,
                    &procedure,
                    stage,
                    seat,
                    &controller,
                    &decision_controller_id,
                    ids.ticket,
                    ids.command,
                    at,
                    &actor_context,
                )?;
                let item = LegalDecisionOutboxItem {
                    sequence,
                    id: format!("outbox:{sequence}"),
                    operation: "create".to_owned(),
                    ticket_id: ids.ticket,
                    create_request_id: ids.create,
                    refresh_request_id: Some(ids.refresh),
                    resolution_request_id: ids.resolution,
                    nested_command_request_id: ids.command,
                    enqueue_expected_revision: None,
                    enqueue_ingress: None,
                    enqueue_outcome_commitment: None,
                    proposal: procedure.proposal.clone(),
                    procedure: procedure_reference(&procedure),
                    stage: procedure.active_stage,
                    round: procedure.round,
                    seat: seat.clone(),
                    decision_controller_id,
                    permission_profile_id,
                    source_boundary: None,
                    controller,
                    command_subject: proposal.sponsor.clone(),
                    draft,
                    knowledge_read_cut: actor_context.read_cut.clone(),
                    knowledge_record_ids: actor_context.knowledge_record_ids.clone(),
                    context_hash: actor_context.context_hash,
                    due_at: at,
                    priority: 0,
                    dispatch: DispatchState::Pending,
                    expires_at: procedure.deadline,
                    acknowledgement: None,
                };
                self.outbox_keys.insert(outbox);
                self.outbox_sequence_by_key.insert(
                    outbox_key(&procedure.id, procedure.active_stage, procedure.round, seat),
                    sequence,
                );
                self.outbox.insert(sequence, item.clone());
                self.pending_outbox_sequences.insert(sequence);
                emitted.push(item);
            }
        }
        Ok(emitted)
    }

    fn prune_staged_actor_contexts(&mut self, changed: &BTreeSet<String>) {
        self.staged_actor_contexts.retain(|key, _| {
            let Some(procedure_id) = key.split('\u{1f}').next() else {
                return false;
            };
            if !changed.contains(procedure_id) {
                return true;
            }
            let Some(procedure) = self.procedures.get(procedure_id) else {
                return false;
            };
            if procedure.closed {
                return false;
            }
            procedure
                .stages
                .get(procedure.active_stage)
                .is_some_and(|stage| {
                    stage.seats.iter().any(|seat| {
                        let current = outbox_key(
                            &procedure.id,
                            procedure.active_stage,
                            procedure.round,
                            seat,
                        );
                        &current == key && !self.outbox_keys.contains(&current)
                    })
                })
        });
    }

    fn refresh_applicability(&mut self, at: SimTime) -> Result<usize, CanwuError> {
        let mut count = 0;
        let dirty_rules = std::mem::take(&mut self.dirty_rules);
        for rule_id in dirty_rules {
            if let Some(stale) = self.applicability_by_rule.remove(&rule_id) {
                for key in stale {
                    if self.applicability.remove(&key).is_some() {
                        count += 1;
                    }
                }
            }
            let Some(rule) = self.rules.get(&rule_id) else {
                return Err(invalid("dirty legal rule is missing"));
            };
            if rule.retired {
                continue;
            }
            let Some(version_ref) = &rule.operative_version else {
                continue;
            };
            let version = self
                .law_versions
                .get(&version_ref.id)
                .ok_or_else(|| invalid("operative law version is missing"))?;
            let mut keys = BTreeSet::new();
            for jurisdiction in &version.jurisdictions {
                for effect in &version.deltas {
                    let id = canonical_hash(
                        "canwu.law.applicability-projection.v1",
                        &(&rule.id, jurisdiction, &effect.id),
                    )?;
                    self.applicability.insert(
                        id.clone(),
                        ApplicabilityResult {
                            query_hash: id.clone(),
                            outcome: ApplicabilityOutcome::Applicable,
                            versions: vec![version_ref.clone()],
                            displaced: Vec::new(),
                            conflicts: Vec::new(),
                            trace: vec![version_ref.clone()],
                            at,
                        },
                    );
                    keys.insert(id);
                    count += 1;
                }
            }
            if !keys.is_empty() {
                self.applicability_by_rule.insert(rule_id, keys);
            }
        }
        Ok(count)
    }

    /// Resolves applicability with the compiled profile's explicit candidate budget.
    pub fn query_applicability_for_plan(
        &self,
        plan: &CompiledLawPlan,
        query: &ApplicabilityQuery,
    ) -> Result<ApplicabilityResult, CanwuError> {
        if query.actor.is_some() {
            return Err(invalid(
                "actor-relative legal applicability must use the host-bound query API",
            ));
        }
        self.query_applicability_prevalidated(plan, query)
    }

    fn query_applicability_prevalidated(
        &self,
        plan: &CompiledLawPlan,
        query: &ApplicabilityQuery,
    ) -> Result<ApplicabilityResult, CanwuError> {
        self.ensure_plan_identity(plan)?;
        if query.read_at > self.last_settled_at || query.event_at > query.read_at {
            return Err(invalid(
                "legal applicability times are outside the settled read boundary",
            ));
        }
        if query.actor.is_some() != query.knowledge_read_cut.is_some()
            || query.facts.keys().ne(query.fact_evidence.keys())
            || (query.actor.is_some() && query.facts.keys().ne(query.fact_knowledge_records.keys()))
            || (query.actor.is_none() && !query.fact_knowledge_records.is_empty())
            || query.fact_evidence.len() > plan.budgets.max_evidence_per_record
        {
            return Err(invalid(
                "legal applicability facts lack actor cut or exact evidence provenance",
            ));
        }
        if query
            .facts
            .keys()
            .any(|predicate| !plan.predicate_by_id.contains_key(predicate))
        {
            return Err(invalid(
                "legal applicability query uses an unknown predicate",
            ));
        }
        let profile = plan
            .applicability_profiles
            .iter()
            .find(|profile| profile.id == query.profile && profile.legal_order == query.legal_order)
            .ok_or_else(|| invalid("legal applicability profile or order is unknown"))?;
        if let Some(jurisdiction) = &query.jurisdiction
            && !plan.jurisdiction_by_id.contains_key(jurisdiction)
        {
            return Err(invalid("legal applicability jurisdiction is unknown"));
        }
        let mut query_work = 0usize;
        consume_query_work(plan, &mut query_work, query.facts.len())?;
        consume_query_work(plan, &mut query_work, profile.pipeline.len())?;
        let reachable_jurisdictions = query
            .jurisdiction
            .as_ref()
            .map(|jurisdiction| {
                reachable_jurisdictions(plan, profile, jurisdiction, &mut query_work)
            })
            .transpose()?;
        self.query_applicability_bounded(
            plan,
            profile,
            query,
            reachable_jurisdictions.as_ref(),
            &mut query_work,
        )
    }

    /// Resolves applicability after checking every asserted fact against host evidence.
    pub fn query_applicability_with_host(
        &self,
        plan: &CompiledLawPlan,
        canwu: &Canwu,
        query: &ApplicabilityQuery,
        knowledge_query: Option<&KnowledgeQuery>,
    ) -> Result<ApplicabilityResult, CanwuError> {
        if query
            .fact_evidence
            .values()
            .any(|reference| !canwu.evidence_exists(reference))
        {
            return Err(invalid(
                "legal applicability fact evidence is unavailable at the host read",
            ));
        }
        match (&query.actor, knowledge_query) {
            (None, None) => {}
            (Some(actor), Some(knowledge_query)) => {
                if knowledge_query.after.is_some()
                    || knowledge_query.limit == 0
                    || usize::try_from(knowledge_query.limit).map_or(true, |limit| {
                        limit > plan.budgets.max_applicability_query_work
                    })
                {
                    return Err(invalid("actor-relative legal knowledge query is unbounded"));
                }
                let result = canwu.admin_query_knowledge(actor.clone(), knowledge_query)?;
                if result.holder != *actor
                    || Some(&result.read_cut) != query.knowledge_read_cut.as_ref()
                    || result.next.is_some()
                {
                    return Err(invalid(
                        "actor-relative legal facts do not match one complete holder read cut",
                    ));
                }
                let host_fact_work = result
                    .records
                    .len()
                    .checked_mul(query.facts.len())
                    .ok_or_else(|| invalid("actor-relative legal fact work overflowed"))?;
                if host_fact_work > plan.budgets.max_applicability_query_work {
                    return Err(invalid(
                        "actor-relative legal fact verification budget exceeded",
                    ));
                }
                for (predicate_id, asserted) in &query.facts {
                    let record_id = query
                        .fact_knowledge_records
                        .get(predicate_id)
                        .ok_or_else(|| invalid("actor-relative legal fact lacks a record"))?;
                    let record = result
                        .records
                        .iter()
                        .find(|record| &record.id == record_id)
                        .ok_or_else(|| invalid("actor-relative legal fact record is absent"))?;
                    let predicate = plan
                        .predicate_by_id
                        .get(predicate_id)
                        .and_then(|key| plan.predicates.get(key.get() as usize))
                        .ok_or_else(|| invalid("actor-relative legal predicate is unknown"))?;
                    let schema = predicate.knowledge_schema.as_ref().ok_or_else(|| {
                        invalid("actor-relative legal predicate has no knowledge schema")
                    })?;
                    let pointer = predicate.payload_pointer.as_deref().ok_or_else(|| {
                        invalid("actor-relative legal predicate has no payload binding")
                    })?;
                    if &record.schema != schema
                        || record
                            .payload
                            .pointer(pointer)
                            .and_then(serde_json::Value::as_bool)
                            != Some(*asserted)
                    {
                        return Err(invalid(
                            "actor-relative legal fact is not derived from its bound record",
                        ));
                    }
                }
            }
            (None, Some(_)) | (Some(_), None) => {
                return Err(invalid(
                    "actor-relative legal applicability requires exactly one holder query",
                ));
            }
        }
        self.query_applicability_prevalidated(plan, query)
    }

    fn query_applicability_bounded(
        &self,
        plan: &CompiledLawPlan,
        profile: &ApplicabilityProfileDefinition,
        query: &ApplicabilityQuery,
        reachable_jurisdictions: Option<&BTreeSet<String>>,
        query_work: &mut usize,
    ) -> Result<ApplicabilityResult, CanwuError> {
        let query_hash = canonical_hash("canwu.law.applicability.v1", query)?;
        let subject = query.subject.as_ref().map(ToString::to_string);
        let actor = query.actor.as_ref().map(|value| match value {
            KnowledgeHolderRef::Person(person) => EntityRef::Person(*person).to_string(),
            KnowledgeHolderRef::Entity(entity) => entity.to_string(),
        });
        let mut candidate_rule_ids = BTreeSet::new();
        let mut received_rule_ids = BTreeSet::new();
        let mut has_indeterminate_scope = false;
        let mut pending_orders = vec![(query.legal_order.clone(), Vec::<usize>::new())];
        let mut traversal_steps = 0usize;
        while let Some((order, reception_path)) = pending_orders.pop() {
            consume_query_work(plan, query_work, 1)?;
            traversal_steps = traversal_steps
                .checked_add(1)
                .ok_or_else(|| invalid("legal succession traversal overflowed"))?;
            if traversal_steps > profile.max_candidates {
                return Err(invalid("legal succession traversal budget exceeded"));
            }
            if let Some(rules) = self.rule_ids_by_order.get(&order) {
                for rule_id in rules {
                    consume_query_work(plan, query_work, 1)?;
                    let continued = reception_path.iter().all(|index| {
                        matching_reception_rule(&self.successions[*index].reception, rule_id)
                            .is_some_and(|rule| rule.action == ReceptionAction::Continue)
                    });
                    if continued {
                        if !candidate_rule_ids.contains(rule_id)
                            && candidate_rule_ids.len() >= profile.max_candidates
                        {
                            return Err(invalid("legal applicability candidate budget exceeded"));
                        }
                        candidate_rule_ids.insert(rule_id.clone());
                        if !reception_path.is_empty() {
                            received_rule_ids.insert(rule_id.clone());
                        }
                    }
                }
            }
            for index in self
                .succession_indexes_by_successor
                .get(&order)
                .into_iter()
                .flatten()
            {
                let succession = &self.successions[*index];
                consume_query_work(plan, query_work, 1)?;
                if succession.effective_at > query.event_at
                    || reception_path.contains(index)
                    || !succession_scope_matches(
                        succession,
                        query,
                        subject.as_deref(),
                        actor.as_deref(),
                    )
                {
                    continue;
                }
                let mut predecessor_path = reception_path.clone();
                predecessor_path.push(*index);
                for predecessor in &succession.predecessors {
                    consume_query_work(plan, query_work, 1)?;
                    if traversal_steps
                        .checked_add(pending_orders.len())
                        .is_none_or(|reserved| reserved >= profile.max_candidates)
                    {
                        return Err(invalid("legal succession traversal budget exceeded"));
                    }
                    pending_orders.push((predecessor.clone(), predecessor_path.clone()));
                }
            }
        }
        if candidate_rule_ids.len() > profile.max_candidates {
            return Err(invalid("legal applicability candidate budget exceeded"));
        }
        let mut latest_by_rule = BTreeMap::<&str, (&LawVersion, Option<&LawVersion>)>::new();
        for rule_id in &candidate_rule_ids {
            let Some(index) = self.law_versions_by_rule.get(rule_id) else {
                continue;
            };
            let mut latest = None;
            let mut materialized = None;
            for reference in index.iter().rev() {
                let version = self
                    .law_versions
                    .get(&reference.id)
                    .ok_or_else(|| invalid("legal applicability version index is dangling"))?;
                // Charge each version and each nested effect before reading it
                // so a low-budget query stops at the first overrun instead of
                // scanning an entire rule history and charging afterward.
                consume_query_work(plan, query_work, 2)?;
                consume_query_work(plan, query_work, version.jurisdictions.len())?;
                for effect in &version.deltas {
                    consume_query_work(plan, query_work, 1)?;
                    consume_query_work(
                        plan,
                        query_work,
                        effect
                            .holders
                            .len()
                            .checked_add(effect.duty_bearers.len())
                            .and_then(|total| total.checked_add(effect.subject_matters.len()))
                            .and_then(|total| total.checked_add(effect.territories.len()))
                            .and_then(|total| total.checked_add(effect.conditions.len()))
                            .and_then(|total| total.checked_add(effect.exceptions.len()))
                            .and_then(|total| total.checked_add(effect.standing.len()))
                            .and_then(|total| total.checked_add(effect.source_refs.len()))
                            .and_then(|total| {
                                total.checked_add(usize::from(effect.forum.is_some()))
                            })
                            .and_then(|total| {
                                total.checked_add(usize::from(effect.remedy_profile.is_some()))
                            })
                            .ok_or_else(|| invalid("legal applicability effect work overflowed"))?,
                    )?;
                }
                if version.adopted_at > query.read_at
                    || !self.version_publicity_satisfied_at(plan, version, query.read_at)
                {
                    continue;
                }
                let mut applies = |version: &LawVersion| {
                    let applies_at_event = version.effective_at <= query.event_at
                        || version.retrospective_from.is_some_and(|from| {
                            from <= query.event_at && version.adopted_at <= query.read_at
                        });
                    if !applies_at_event {
                        return false;
                    }
                    if version.applicability_profile != query.profile
                        && !received_rule_ids.contains(rule_id)
                    {
                        return false;
                    }
                    let mut effect_is_indeterminate = false;
                    let scope_matches = version.deltas.is_empty()
                        || version.deltas.iter().any(|effect| {
                            let subject_matches = subject.as_ref().is_none_or(|subject| {
                                effect.holders.is_empty()
                                    || effect.holders.iter().any(|holder| holder == subject)
                            });
                            let actor_matches = actor.as_ref().is_none_or(|actor| {
                                effect.duty_bearers.is_empty()
                                    || effect.duty_bearers.iter().any(|holder| holder == actor)
                            });
                            let territory_matches = query.territory.is_none_or(|territory| {
                                effect.territories.is_empty()
                                    || effect.territories.contains(&territory)
                            });
                            let subject_matter_matches =
                                query.subject_matter.as_ref().is_none_or(|matter| {
                                    effect.subject_matters.is_empty()
                                        || effect.subject_matters.contains(matter)
                                });
                            let base_matches = subject_matches
                                && actor_matches
                                && territory_matches
                                && subject_matter_matches;
                            if !base_matches {
                                return false;
                            }
                            match predicate_scope_match(effect, &query.facts) {
                                PredicateScopeMatch::Applies => true,
                                PredicateScopeMatch::Excluded => false,
                                PredicateScopeMatch::Indeterminate => {
                                    effect_is_indeterminate = true;
                                    false
                                }
                            }
                        });
                    has_indeterminate_scope |= effect_is_indeterminate;
                    let jurisdiction_matches = reachable_jurisdictions.is_none_or(|reachable| {
                        version.jurisdictions.is_empty()
                            || version
                                .jurisdictions
                                .iter()
                                .any(|jurisdiction| reachable.contains(jurisdiction))
                    });
                    profile.pipeline.iter().all(|step| match step.as_str() {
                        "scope" => scope_matches,
                        "jurisdiction" => jurisdiction_matches,
                        "validity" | "conflict" => true,
                        _ => false,
                    })
                };
                if applies(version) {
                    latest.get_or_insert(version);
                    if materialized.is_none() && materializes_legal_effect(version) {
                        materialized = Some(version);
                    }
                    if latest.is_some() && materialized.is_some() {
                        break;
                    }
                }
            }
            if let Some(latest) = latest {
                latest_by_rule.insert(latest.rule.as_str(), (latest, materialized));
            }
        }
        let mut displaced = Vec::new();
        let mut versions = Vec::new();
        for (latest, materialized) in latest_by_rule.into_values() {
            let latest_reference = law_version_reference(latest);
            if latest.disposition == OperativeDisposition::Operative {
                versions.push(latest_reference);
            } else {
                displaced.push(latest_reference.clone());
                if matches!(
                    latest.disposition,
                    OperativeDisposition::Claimed
                        | OperativeDisposition::Purported
                        | OperativeDisposition::Contested
                ) && let Some(materialized) = materialized
                    && materialized.disposition == OperativeDisposition::Operative
                {
                    let materialized_reference = law_version_reference(materialized);
                    if materialized_reference != latest_reference {
                        versions.push(materialized_reference);
                    }
                }
            }
        }
        versions.sort();
        displaced.sort();
        let claim_ids = versions
            .iter()
            .chain(&displaced)
            .map(|reference| reference.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut candidate_conflict_ids = BTreeSet::new();
        for version in &claim_ids {
            if let Some(ids) = self.conflict_ids_by_version.get(*version) {
                consume_query_work(plan, query_work, ids.len())?;
                candidate_conflict_ids.extend(ids.iter().cloned());
            }
        }
        let mut conflicts = Vec::new();
        for id in candidate_conflict_ids {
            let conflict = self
                .conflicts
                .get(&id)
                .ok_or_else(|| invalid("legal conflict query index is dangling"))?;
            consume_query_work(plan, query_work, conflict.versions.len())?;
            let active_in_time = conflict.recorded_at <= query.read_at
                && conflict.effective_from <= query.event_at
                && conflict
                    .effective_until
                    .is_none_or(|until| query.event_at <= until);
            let jurisdiction_matches = conflict
                .jurisdiction
                .as_ref()
                .is_none_or(|jurisdiction| query.jurisdiction.as_ref() == Some(jurisdiction));
            if active_in_time
                && jurisdiction_matches
                && conflict
                    .versions
                    .iter()
                    .all(|reference| claim_ids.contains(reference.id.as_str()))
            {
                conflicts.push(id);
            }
        }
        conflicts.sort();
        let mut resolved_claim_ids = BTreeSet::new();
        let mut resolved_governing = BTreeSet::new();
        let mut resolved_displaced = BTreeSet::new();
        for conflict_id in &conflicts {
            let conflict = &self.conflicts[conflict_id];
            if conflict.resolution != ApplicabilityOutcome::Displaced {
                continue;
            }
            resolved_claim_ids.extend(
                conflict
                    .versions
                    .iter()
                    .map(|reference| reference.id.clone()),
            );
            resolved_governing.extend(conflict.governing_versions.iter().cloned());
            resolved_displaced.extend(conflict.displaced_versions.iter().cloned());
        }
        let has_conflicting_partitions = resolved_governing
            .iter()
            .any(|reference| resolved_displaced.contains(reference));
        if has_conflicting_partitions {
            resolved_claim_ids.clear();
        } else {
            versions.retain(|reference| !resolved_claim_ids.contains(&reference.id));
            displaced.retain(|reference| !resolved_claim_ids.contains(&reference.id));
            versions.extend(resolved_governing);
            displaced.extend(resolved_displaced);
        }
        versions.sort();
        versions.dedup();
        displaced.sort();
        displaced.dedup();
        let has_disputed_validity = displaced.iter().any(|reference| {
            !resolved_claim_ids.contains(&reference.id)
                && self.law_versions.get(&reference.id).is_some_and(|version| {
                    matches!(
                        version.disposition,
                        OperativeDisposition::Claimed
                            | OperativeDisposition::Purported
                            | OperativeDisposition::Contested
                    )
                })
        });
        let outcome = if versions.is_empty() {
            if displaced.is_empty() {
                if has_indeterminate_scope {
                    ApplicabilityOutcome::Indeterminate
                } else {
                    ApplicabilityOutcome::NotApplicable
                }
            } else if has_disputed_validity {
                ApplicabilityOutcome::Contested
            } else {
                ApplicabilityOutcome::Displaced
            }
        } else if has_conflicting_partitions
            || has_disputed_validity
            || conflicts.iter().any(|id| {
                self.conflicts
                    .get(id)
                    .is_some_and(|conflict| conflict.resolution == ApplicabilityOutcome::Contested)
            })
        {
            ApplicabilityOutcome::Contested
        } else if has_indeterminate_scope {
            ApplicabilityOutcome::Indeterminate
        } else {
            ApplicabilityOutcome::Applicable
        };
        let mut trace = versions.clone();
        trace.extend(displaced.iter().cloned());
        trace.sort();
        trace.dedup();
        Ok(ApplicabilityResult {
            query_hash,
            outcome,
            versions: versions.clone(),
            displaced,
            conflicts,
            trace,
            at: query.read_at,
        })
    }

    fn version_publicity_satisfied_at(
        &self,
        plan: &CompiledLawPlan,
        version: &LawVersion,
        read_at: SimTime,
    ) -> bool {
        let Some(source) = self.sources.get(&version.source.id) else {
            return false;
        };
        let Some(proposal) = self.proposals.get(&source.proposal.id) else {
            return false;
        };
        let Some(profile) = plan
            .source_profile_by_id
            .get(&proposal.source_profile)
            .and_then(|key| plan.source_profiles.get(key.get() as usize))
        else {
            return false;
        };
        match profile.publicity_policy {
            PublicityPolicy::ValidityCondition | PublicityPolicy::EffectivenessCondition => {
                let Some(reference) = proposal.publicity.as_ref() else {
                    return false;
                };
                self.publicity_events
                    .get(&reference.id)
                    .is_some_and(|event| {
                        reference == &local_ref("publicity", &event.id)
                            && event.proposal == source.proposal
                            && event.at <= read_at
                            && match profile.publicity_policy {
                                PublicityPolicy::ValidityCondition => {
                                    event.at <= version.adopted_at
                                        && source.publicity_event.as_ref() == Some(reference)
                                        && source.promulgated_at == Some(event.at)
                                        && version.promulgated_at == Some(event.at)
                                }
                                PublicityPolicy::EffectivenessCondition => {
                                    event.at <= version.effective_at
                                        && match source.publicity_event.as_ref() {
                                            Some(source_reference) => {
                                                source_reference == reference
                                                    && source.promulgated_at == Some(event.at)
                                                    && version.promulgated_at == Some(event.at)
                                            }
                                            None => {
                                                source.promulgated_at.is_none()
                                                    && version.promulgated_at.is_none()
                                                    && proposal
                                                        .adopted_at
                                                        .is_some_and(|adopted| adopted <= event.at)
                                            }
                                        }
                                }
                                PublicityPolicy::NotRequired | PublicityPolicy::EvidenceOnly => {
                                    false
                                }
                            }
                    })
            }
            PublicityPolicy::NotRequired | PublicityPolicy::EvidenceOnly => true,
        }
    }

    pub fn record_case(
        &mut self,
        plan: &CompiledLawPlan,
        case: LegalCase,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        if nested_case_items(&case)
            .is_none_or(|items| items > plan.budgets.max_nested_items_per_record)
        {
            return Err(invalid("legal case nested-item budget exceeded"));
        }
        validate_runtime_text_budget(&case, plan.budgets.max_text_bytes)?;
        if self.cases.len() >= plan.budgets.max_cases
            || self.cases.contains_key(&case.id)
            || case.allegations.len() > plan.budgets.max_evidence_per_record
        {
            return Err(invalid("legal case budget or identity violation"));
        }
        let forum = plan
            .forum_by_id
            .get(&case.forum)
            .and_then(|key| plan.forums.get(key.get() as usize))
            .ok_or_else(|| invalid("legal case forum profile is unknown"))?;
        if case.claims.is_empty()
            || case.issues.is_empty()
            || case.subject_matters.is_empty()
            || case.deadline < self.last_settled_at
            || !scope_covers(&forum.legal_orders, &case.legal_order)
            || case
                .subject_matters
                .iter()
                .any(|matter| !scope_covers(&forum.subject_matters, matter))
            || !forum.proof_profiles.contains(&case.proof_profile)
            || case
                .standing
                .as_ref()
                .is_none_or(|standing| !forum.standing_profiles.contains(standing))
            || case.remedies.is_empty()
            || case
                .remedies
                .iter()
                .any(|remedy| !forum.remedy_profiles.contains(remedy))
        {
            return Err(invalid("legal case is outside its compiled forum contract"));
        }
        self.reserve_state_growth(Self::encoded_growth(&case, 4)?)?;
        self.cases.insert(case.id.clone(), case);
        Ok(())
    }

    /// Records a separate immutable publication act for one admitted proposal.
    pub fn record_publicity(
        &mut self,
        plan: &CompiledLawPlan,
        event: LegalPublicityEvent,
    ) -> Result<(), CanwuError> {
        self.record_publicity_at(plan, event, self.last_settled_at)
    }

    pub(crate) fn record_publicity_at(
        &mut self,
        plan: &CompiledLawPlan,
        mut event: LegalPublicityEvent,
        occurred_at: SimTime,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        validate_runtime_text_budget(&event, plan.budgets.max_text_bytes)?;
        event.scope.sort();
        event.scope.dedup();
        event.evidence.sort();
        event.evidence.dedup();
        if self.publicity_events.len() >= plan.budgets.max_sources
            || self.publicity_events.contains_key(&event.id)
            || event.proposal.kind != "proposal"
            || event.at != occurred_at
            || event.at < self.last_settled_at
            || event.signal_kind.is_empty()
            || event.medium.is_empty()
            || event.scope.is_empty()
            || event.scope.len() > plan.budgets.max_jurisdictions_per_proposal
            || event.evidence.is_empty()
            || event.evidence.len() > plan.budgets.max_evidence_per_record
        {
            return Err(invalid(
                "legal publicity event budget or content is invalid",
            ));
        }
        let proposal = self
            .proposals
            .get(&event.proposal.id)
            .filter(|proposal| event.proposal == local_ref("proposal", &proposal.id))
            .ok_or_else(|| invalid("legal publicity references a non-exact proposal"))?;
        let profile = plan
            .source_profile_by_id
            .get(&proposal.source_profile)
            .and_then(|key| plan.source_profiles.get(key.get() as usize))
            .ok_or_else(|| invalid("legal publicity source profile is missing"))?;
        if profile.publicity_policy == PublicityPolicy::NotRequired {
            return Err(invalid("legal source profile does not admit publicity"));
        }
        if profile.publicity_signal_kind.as_ref() != Some(&event.signal_kind) {
            return Err(invalid(
                "legal publicity event does not match its compiled signal provider",
            ));
        }
        if event.scope != proposal.jurisdictions {
            return Err(invalid(
                "legal publicity scope must exactly cover proposal jurisdictions",
            ));
        }
        let delayed_adoption = proposal.status == ProposalStatus::Adopted
            && profile.publicity_policy == PublicityPolicy::EffectivenessCondition;
        if proposal.publicity.is_some()
            || matches!(proposal.status, ProposalStatus::Rejected)
            || (proposal.status == ProposalStatus::Adopted && !delayed_adoption)
        {
            return Err(invalid("legal proposal cannot accept this publicity event"));
        }
        if delayed_adoption {
            if event.at > proposal.effective_at {
                return Err(invalid(
                    "effectiveness publicity must occur no later than the effective time",
                ));
            }
            let source_reference = proposal
                .source_version
                .as_ref()
                .ok_or_else(|| invalid("adopted proposal lacks its source link"))?;
            let version_reference = proposal
                .law_version
                .as_ref()
                .ok_or_else(|| invalid("adopted proposal lacks its law-version link"))?;
            self.sources
                .get(&source_reference.id)
                .filter(|source| {
                    source_reference == &local_ref("source_version", &source.id)
                        && source.proposal == local_ref("proposal", &proposal.id)
                        && source.publicity_event.is_none()
                })
                .ok_or_else(|| invalid("adopted proposal source link is not exact"))?;
            let version = self
                .law_versions
                .get(&version_reference.id)
                .filter(|version| {
                    version_reference == &law_version_reference(version)
                        && version.source == *source_reference
                        && version.promulgated_at.is_none()
                })
                .cloned()
                .ok_or_else(|| invalid("adopted proposal law-version link is not exact"))?;
            let event_ref = local_ref("publicity", &event.id);
            let proposal_id = proposal.id.clone();
            let evidence = event.evidence.iter().cloned().collect::<BTreeSet<_>>();
            self.reserve_state_growth(Self::encoded_growth(&event, 4)?)?;
            self.publicity_events
                .insert(event.id.clone(), event.clone());
            self.proposals
                .get_mut(&proposal_id)
                .ok_or_else(|| invalid("validated publicity proposal disappeared"))?
                .publicity = Some(event_ref);
            let reference = law_version_reference(&version);
            if version.effective_at > occurred_at {
                let rule = self
                    .rules
                    .get_mut(&version.rule)
                    .ok_or_else(|| invalid("delayed publicity version has no rule"))?;
                rule.scheduled_versions.push(reference.clone());
                rule.scheduled_versions.sort();
                rule.scheduled_versions.dedup();
                let scheduled = self
                    .scheduled_versions_by_time
                    .entry(version.effective_at)
                    .or_default();
                scheduled.push(reference.clone());
                scheduled.sort();
                scheduled.dedup();
                for dependency in &version.cultural_dependencies {
                    if dependency.kind == CulturalDependencyKind::LiveLevel {
                        self.scheduled_live_dependencies
                            .entry(dependency.target.clone())
                            .or_default()
                            .insert(reference.clone());
                    }
                }
            } else {
                let replaced = self
                    .rules
                    .get(&version.rule)
                    .filter(|rule| !rule.retired)
                    .and_then(|rule| rule.operative_version.as_ref())
                    .map(|current| self.law_version_identity_evidence_dependencies(current));
                let rule = self
                    .rules
                    .get_mut(&version.rule)
                    .ok_or_else(|| invalid("delayed publicity version has no rule"))?;
                rule.operative_version = Some(reference.clone());
                rule.effects = version.deltas.clone();
                rule.retired = retires_rule(version.operation);
                if let Some(position) = rule
                    .scheduled_versions
                    .iter()
                    .position(|item| item == &reference)
                {
                    rule.scheduled_versions.remove(position);
                }
                self.dirty_rules.insert(version.rule.clone());
                if let Some(dependencies) = replaced {
                    self.remove_identity_evidence_component(dependencies)?;
                }
                if retires_rule(version.operation) {
                    self.remove_identity_evidence_component(
                        self.law_version_identity_evidence_dependencies(&reference),
                    )?;
                }
            }
            self.dirty_rules.insert(version.rule.clone());
            self.add_identity_evidence_component(evidence)?;
            return Ok(());
        }
        if proposal.status == ProposalStatus::Adopted {
            return Err(invalid(
                "adopted proposal requires effectiveness publicity policy",
            ));
        }
        let event_ref = local_ref("publicity", &event.id);
        let proposal_id = event.proposal.id.clone();
        let evidence = event
            .evidence
            .iter()
            .cloned()
            .collect::<BTreeSet<EvidenceRef>>();
        if evidence.iter().any(|reference| {
            self.retained_evidence_dependency_counts
                .get(reference)
                .is_some_and(|count| *count == usize::MAX)
        }) {
            return Err(invalid("legal publicity evidence dependency overflowed"));
        }
        self.reserve_state_growth(Self::encoded_growth(&event, 4)?)?;
        self.publicity_events.insert(event.id.clone(), event);
        let proposal = self
            .proposals
            .get_mut(&proposal_id)
            .ok_or_else(|| invalid("validated publicity proposal disappeared"))?;
        proposal.publicity = Some(event_ref);
        self.add_identity_evidence_component(evidence)?;
        Ok(())
    }
    pub fn record_finding(
        &mut self,
        plan: &CompiledLawPlan,
        finding: LegalFindingVersion,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        validate_runtime_text_budget(&finding, plan.budgets.max_text_bytes)?;
        let case = self
            .cases
            .get(&finding.case_id)
            .ok_or_else(|| invalid("legal finding references an unknown case"))?;
        if self.findings.len() >= plan.budgets.max_findings
            || self.findings.contains_key(&finding.id)
            || !case.issues.contains(&finding.issue)
            || finding.burden != case.proof_profile
            || finding.at > case.deadline
            || finding.evidence.len() > plan.budgets.max_evidence_per_record
            || finding.predecessor.as_ref().is_some_and(|reference| {
                self.findings.get(&reference.id).is_none_or(|predecessor| {
                    reference != &local_ref("finding", &predecessor.id)
                        || predecessor.case_id != finding.case_id
                        || predecessor.issue != finding.issue
                        || predecessor.at > finding.at
                })
            })
        {
            return Err(invalid(
                "legal finding budget, identity, or topology violation",
            ));
        }
        self.reserve_state_growth(Self::encoded_growth(&finding, 4)?)?;
        self.findings.insert(finding.id.clone(), finding);
        Ok(())
    }
    pub fn record_ruling(
        &mut self,
        plan: &CompiledLawPlan,
        mut ruling: LegalRulingVersion,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        if nested_ruling_items(&ruling)
            .is_none_or(|items| items > plan.budgets.max_nested_items_per_record)
        {
            return Err(invalid("legal ruling nested-item budget exceeded"));
        }
        validate_runtime_text_budget(&ruling, plan.budgets.max_text_bytes)?;
        canonicalize_ruling(&mut ruling);
        if self.rulings.len() >= plan.budgets.max_rulings
            || self.rulings.contains_key(&ruling.id)
            || !plan.institution_by_id.contains_key(&ruling.institution)
            || ruling.evidence.len() > plan.budgets.max_evidence_per_record
        {
            return Err(invalid(
                "legal ruling budget, identity, evidence, or institution violation",
            ));
        }
        let case = self
            .cases
            .get(&ruling.case_id)
            .ok_or_else(|| invalid("legal ruling references unknown case"))?;
        if ruling.findings.iter().any(|reference| {
            self.findings
                .get(&reference.id)
                .is_none_or(|finding| reference != &local_ref("finding", &finding.id))
        }) || ruling.sources.iter().any(|reference| {
            self.sources
                .get(&reference.id)
                .is_none_or(|source| reference != &local_ref("source_version", &source.id))
        }) || ruling.resolved_versions.iter().any(|reference| {
            self.law_versions
                .get(&reference.id)
                .is_none_or(|version| reference != &law_version_reference(version))
        }) || ruling
            .selected_versions
            .iter()
            .any(|reference| ruling.resolved_versions.binary_search(reference).is_err())
            || ruling.predecessors.iter().any(|reference| {
                self.rulings.get(&reference.id).is_none_or(|predecessor| {
                    reference != &local_ref("ruling", &predecessor.id)
                        || predecessor.case_id != ruling.case_id
                        || predecessor.effective_from > ruling.effective_from
                })
            })
        {
            return Err(invalid("legal ruling references unknown exact records"));
        }
        if ruling
            .effective_until
            .is_some_and(|until| until < ruling.effective_from)
        {
            return Err(invalid("legal ruling effective interval is inverted"));
        }
        {
            let forum = plan
                .forum_by_id
                .get(&case.forum)
                .and_then(|key| plan.forums.get(key.get() as usize))
                .ok_or_else(|| invalid("legal ruling forum profile is missing"))?;
            let findings_match = !ruling.findings.is_empty()
                && ruling.findings.iter().all(|reference| {
                    self.findings.get(&reference.id).is_some_and(|finding| {
                        finding.case_id == case.id && ruling.issues.contains(&finding.issue)
                    })
                });
            let institution = plan
                .institutions
                .iter()
                .find(|institution| institution.id == ruling.institution)
                .ok_or_else(|| invalid("legal ruling institution is missing"))?;
            let competence_matches = institution.competences.iter().any(|competence| {
                competence.can_adjudicate
                    && scope_covers(&competence.forums, &case.forum)
                    && scope_covers(&competence.legal_orders, &case.legal_order)
                    && scope_covers(&competence.jurisdictions, &forum.jurisdiction)
                    && case
                        .subject_matters
                        .iter()
                        .all(|matter| scope_covers(&competence.subject_matters, matter))
                    && ruling
                        .scope
                        .iter()
                        .all(|scope| scope_covers(&competence.jurisdictions, scope))
            });
            if ruling.issues.is_empty()
                || ruling.scope.is_empty()
                || ruling.findings.is_empty()
                || ruling
                    .issues
                    .iter()
                    .any(|issue| !case.issues.contains(issue))
                || !findings_match
                || !forum.institutions.contains(&ruling.institution)
                || !competence_matches
                || ruling.remedy.as_ref().is_some_and(|remedy| {
                    !case.remedies.contains(remedy) || !forum.remedy_profiles.contains(remedy)
                })
                || ruling
                    .precedent_profile
                    .as_ref()
                    .is_some_and(|precedent| !forum.precedent_profiles.contains(precedent))
            {
                return Err(invalid(
                    "legal ruling is outside compiled adjudicative competence",
                ));
            }
        }
        self.reserve_state_growth(Self::encoded_growth(&ruling, 4)?)?;
        self.rulings.insert(ruling.id.clone(), ruling);
        Ok(())
    }

    pub fn record_conflict(
        &mut self,
        plan: &CompiledLawPlan,
        mut conflict: LegalConflict,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        if nested_conflict_items(&conflict)
            .is_none_or(|items| items > plan.budgets.max_nested_items_per_record)
        {
            return Err(invalid("legal conflict nested-item budget exceeded"));
        }
        validate_runtime_text_budget(&conflict, plan.budgets.max_text_bytes)?;
        if self.conflicts.len() >= plan.budgets.max_conflicts
            || self.conflicts.contains_key(&conflict.id)
        {
            return Err(invalid("legal conflict budget or identity violation"));
        }
        conflict.versions.sort();
        conflict.versions.dedup();
        conflict.governing_versions.sort();
        conflict.governing_versions.dedup();
        conflict.displaced_versions.sort();
        conflict.displaced_versions.dedup();
        conflict.trace.sort();
        conflict.trace.dedup();
        if conflict.versions.len() < 2
            || conflict.versions.len() > plan.budgets.max_applicability_query_work
            || conflict.rationale.trim().is_empty()
            || conflict.recorded_at > self.last_settled_at
            || conflict
                .effective_until
                .is_some_and(|until| until < conflict.effective_from)
        {
            return Err(invalid(
                "legal conflict requires bounded exact versions, valid time, and rationale",
            ));
        }
        if let Some(ruling) = &conflict.ruling
            && self
                .rulings
                .get(&ruling.id)
                .is_none_or(|value| ruling != &local_ref("ruling", &value.id))
        {
            return Err(invalid("legal conflict references an unknown ruling"));
        }
        if conflict.versions.iter().any(|reference| {
            self.law_versions
                .get(&reference.id)
                .is_none_or(|version| reference != &law_version_reference(version))
        }) {
            return Err(invalid("legal conflict references a non-exact law version"));
        }
        if conflict
            .jurisdiction
            .as_ref()
            .is_some_and(|id| !plan.jurisdiction_by_id.contains_key(id))
        {
            return Err(invalid("legal conflict jurisdiction is unknown"));
        }
        let basis_is_authorized = conflict.versions.iter().all(|reference| {
            self.law_versions
                .get(&reference.id)
                .and_then(|version| self.rules.get(&version.rule))
                .and_then(|rule| plan.order_by_id.get(&rule.legal_order))
                .and_then(|key| plan.orders.get(key.get() as usize))
                .and_then(|order| plan.precedence_profile_by_id.get(&order.precedence_profile))
                .and_then(|key| plan.precedence_profiles.get(key.get() as usize))
                .is_some_and(|profile| profile.ordered_bases.contains(&conflict.basis))
        });
        if !basis_is_authorized {
            return Err(invalid("legal conflict basis is outside order precedence"));
        }
        let mut resolved_union = conflict.governing_versions.clone();
        resolved_union.extend(conflict.displaced_versions.iter().cloned());
        resolved_union.sort();
        resolved_union.dedup();
        let overlap = conflict
            .governing_versions
            .iter()
            .any(|reference| conflict.displaced_versions.contains(reference));
        match conflict.resolution {
            ApplicabilityOutcome::Displaced => {
                if conflict.governing_versions.is_empty()
                    || conflict.displaced_versions.is_empty()
                    || overlap
                    || resolved_union != conflict.versions
                {
                    return Err(invalid(
                        "resolved legal conflict must partition governing and displaced versions",
                    ));
                }
            }
            ApplicabilityOutcome::Contested => {
                if !conflict.governing_versions.is_empty()
                    || !conflict.displaced_versions.is_empty()
                {
                    return Err(invalid(
                        "contested legal conflict cannot claim a resolved version partition",
                    ));
                }
            }
            _ => return Err(invalid("legal conflict has an invalid resolution state")),
        }
        if conflict.basis == ConflictResolutionBasis::Temporal
            && conflict.resolution == ApplicabilityOutcome::Displaced
        {
            let winner = conflict
                .versions
                .iter()
                .max_by_key(|reference| {
                    let version = &self.law_versions[&reference.id];
                    (
                        version.effective_at,
                        version.adopted_at,
                        version.legal_ordinal,
                        version.id.as_str(),
                    )
                })
                .ok_or_else(|| invalid("temporal conflict versions disappeared"))?;
            if conflict.governing_versions.as_slice() != [winner.clone()] {
                return Err(invalid("temporal conflict partition has the wrong winner"));
            }
        }
        if conflict.resolution == ApplicabilityOutcome::Displaced
            && conflict.basis != ConflictResolutionBasis::Temporal
            && (conflict.ruling.is_none() || conflict.jurisdiction.is_none())
        {
            return Err(invalid(
                "non-temporal conflict resolution requires a competent exact ruling and jurisdiction",
            ));
        }
        if let Some(reference) = &conflict.ruling {
            let ruling = self
                .rulings
                .get(&reference.id)
                .filter(|ruling| reference == &local_ref("ruling", &ruling.id))
                .ok_or_else(|| invalid("legal conflict ruling is not exact"))?;
            if ruling.disposition != OperativeDisposition::Operative
                || ruling.resolved_versions != conflict.versions
                || ruling.selected_versions != conflict.governing_versions
                || ruling.effective_from > conflict.effective_from
                || match (ruling.effective_until, conflict.effective_until) {
                    (Some(ruling_until), Some(conflict_until)) => ruling_until < conflict_until,
                    (Some(_), None) => true,
                    (None, _) => false,
                }
                || conflict
                    .jurisdiction
                    .as_ref()
                    .is_some_and(|jurisdiction| !scope_covers(&ruling.scope, jurisdiction))
            {
                return Err(invalid(
                    "legal conflict partition does not match its exact ruling",
                ));
            }
        }
        self.reserve_state_growth(Self::encoded_growth(&conflict, 8)?)?;
        for reference in &conflict.versions {
            self.conflict_ids_by_version
                .entry(reference.id.clone())
                .or_default()
                .insert(conflict.id.clone());
        }
        self.conflicts.insert(conflict.id.clone(), conflict);
        Ok(())
    }
    fn record_succession(
        &mut self,
        mut succession: LegalOrderSuccession,
    ) -> Result<(), CanwuError> {
        validate_runtime_text_budget(&succession, self.budgets.max_text_bytes)?;
        if self
            .successions
            .iter()
            .any(|candidate| candidate.id == succession.id)
        {
            return Err(invalid("duplicate legal order succession"));
        }
        canonicalize_succession(&mut succession)?;
        self.reserve_state_growth(Self::encoded_growth(&succession, 8)?)?;
        self.successions.push(succession);
        self.successions.sort_by(|a, b| a.id.cmp(&b.id));
        self.rebuild_succession_index();
        Ok(())
    }

    pub fn record_succession_for_plan(
        &mut self,
        plan: &CompiledLawPlan,
        succession: LegalOrderSuccession,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        if self.successions.len() >= plan.budgets.max_successions {
            return Err(invalid("legal succession budget exhausted"));
        }
        if succession.predecessors.is_empty()
            || succession.successors.is_empty()
            || succession.reception.is_empty()
            || succession
                .reception
                .iter()
                .any(|rule| !valid_reception_rule(rule))
            || succession.reception.iter().any(|rule| {
                rule.transform.as_ref().is_some_and(|transform| {
                    !plan.clauses.iter().any(|clause| &clause.id == transform)
                })
            })
            || succession.evidence.len() > plan.budgets.max_evidence_per_record
            || succession.effective_at < self.last_settled_at
            || succession
                .predecessors
                .iter()
                .any(|order| succession.successors.contains(order))
        {
            return Err(invalid(
                "legal succession requires bounded evidence, monotonic time, and disjoint orders",
            ));
        }
        if succession
            .predecessors
            .iter()
            .chain(&succession.successors)
            .any(|order| !plan.order_by_id.contains_key(order))
        {
            return Err(invalid(
                "legal succession references an unknown legal order",
            ));
        }
        self.record_succession(succession)
    }

    /// Retire one exact culture generation while retaining enacted legal records.
    pub fn retire_cultural_target_for_plan(
        &mut self,
        plan: &CompiledLawPlan,
        target: &CulturalTargetGenerationRef,
        at: SimTime,
        reason: impl Into<String>,
    ) -> Result<(), CanwuError> {
        self.ensure_plan_identity(plan)?;
        self.retire_cultural_target_with_limit(
            target,
            at,
            reason.into(),
            plan.budgets.max_retirements,
        )
    }

    fn retire_cultural_target_with_limit(
        &mut self,
        target: &CulturalTargetGenerationRef,
        at: SimTime,
        reason: String,
        limit: usize,
    ) -> Result<(), CanwuError> {
        if self.retired_cultural_targets.contains(target) {
            return Ok(());
        }
        validate_runtime_text_budget(&(&target, &reason), self.budgets.max_text_bytes)?;
        if at < self.last_settled_at {
            return Err(invalid("culture retirement cannot precede legal state"));
        }
        if self.retirements.len() >= limit {
            return Err(invalid("legal retirement budget exhausted"));
        }
        let dependency_checks = self
            .open_procedures
            .len()
            .checked_add(self.pending_intents.len())
            .and_then(|value| value.checked_add(self.outbox.len()))
            .and_then(|value| value.checked_add(self.rules.len()))
            .ok_or_else(|| invalid("legal retirement dependency work overflowed"))?;
        if dependency_checks > self.budgets.max_retirement_dependency_records {
            return Err(invalid(
                "legal retirement dependency-check budget exhausted",
            ));
        }
        let proposal_depends_on_target = |proposal_id: &str| {
            self.proposals.get(proposal_id).is_some_and(|proposal| {
                proposal
                    .cultural_dependencies
                    .iter()
                    .any(|dependency| &dependency.target == target)
            })
        };
        let open_procedure_dependency = self.open_procedures.iter().any(|procedure_id| {
            self.procedures
                .get(procedure_id)
                .is_some_and(|procedure| proposal_depends_on_target(&procedure.proposal.id))
        });
        let pending_intent_dependency = self
            .pending_intents
            .values()
            .any(|intent| proposal_depends_on_target(&intent.proposal.id));
        let live_outbox_dependency = self.outbox.values().any(|item| {
            matches!(
                item.dispatch,
                DispatchState::Pending | DispatchState::Enqueued
            ) && proposal_depends_on_target(&item.proposal.id)
        });
        let operative_live_level_dependency = self.rules.values().any(|rule| {
            !rule.retired
                && rule
                    .operative_version
                    .as_ref()
                    .and_then(|reference| self.law_versions.get(&reference.id))
                    .is_some_and(|version| {
                        version.cultural_dependencies.iter().any(|dependency| {
                            dependency.kind == CulturalDependencyKind::LiveLevel
                                && &dependency.target == target
                        })
                    })
        });
        let scheduled_live_level_dependency = self
            .scheduled_live_dependencies
            .get(target)
            .is_some_and(|versions| !versions.is_empty());
        if open_procedure_dependency
            || pending_intent_dependency
            || live_outbox_dependency
            || operative_live_level_dependency
            || scheduled_live_level_dependency
        {
            return Err(invalid(
                "live legal decision dependency blocks culture retirement",
            ));
        }
        self.reserve_state_growth(Self::encoded_growth(&(&target, &reason), 4)?)?;
        self.retired_cultural_targets.insert(target.clone());
        let record_id = format!("{}@{}", target.target, target.generation);
        self.retirements.push(LegalRetirement {
            id: format!("retirement:{}:{}", record_id, self.boundary_index),
            kind: "culture_target".to_owned(),
            record: DomainRecordRef::new(PLUGIN_NAMESPACE, "cultural_target", &record_id),
            cultural_target: Some(target.clone()),
            retired_at: at,
            successor: None,
            reason,
            evidence: Vec::new(),
        });
        Ok(())
    }

    pub(crate) fn retire_cultural_target_from_ingress(
        &mut self,
        plan: &CompiledLawPlan,
        target: &CulturalTargetGenerationRef,
        at: SimTime,
        reason: String,
        ingress: EvidenceRef,
    ) -> Result<(), CanwuError> {
        let before = self.retirements.len();
        self.retire_cultural_target_with_limit(target, at, reason, plan.budgets.max_retirements)?;
        if self.retirements.len() > before
            && let Some(retirement) = self.retirements.last_mut()
        {
            retirement.evidence = vec![ingress];
        }
        Ok(())
    }

    fn rebuild_succession_index(&mut self) {
        self.succession_indexes_by_successor.clear();
        for (index, succession) in self.successions.iter().enumerate() {
            for successor in &succession.successors {
                self.succession_indexes_by_successor
                    .entry(successor.clone())
                    .or_default()
                    .push(index);
            }
        }
    }

    pub fn to_record_draft(&self) -> Result<DomainRecordDraft, CanwuError> {
        let reference = crate::legal_runtime_reference().into_untyped();
        let payload = self.persisted_payload()?;
        let encoded_bytes = serde_json::to_vec(&payload)
            .map_err(|error| invalid(format!("legal runtime cannot be encoded: {error}")))?
            .len();
        if encoded_bytes > self.reserved_state_bytes
            || encoded_bytes > self.budgets.max_state_bytes
            || encoded_bytes > self.budgets.max_memory_bytes
        {
            return Err(invalid("legal persisted payload budget exceeded"));
        }
        Ok(DomainRecordDraft::new(reference, payload))
    }

    fn persisted_payload(&self) -> Result<serde_json::Value, CanwuError> {
        let mut payload = serde_json::to_value(self)
            .map_err(|error| invalid(format!("legal runtime cannot be encoded: {error}")))?;
        let declaration =
            serde_json::to_value(self.identity_evidence_dependencies()).map_err(|error| {
                invalid(format!(
                    "legal evidence dependencies cannot be encoded: {error}"
                ))
            })?;
        payload
            .as_object_mut()
            .ok_or_else(|| invalid("legal runtime payload must be an object"))?
            .insert(IDENTITY_EVIDENCE_DEPENDENCIES_FIELD.to_owned(), declaration);
        Ok(payload)
    }

    fn persisted_payload_len(&self) -> Result<usize, CanwuError> {
        serde_json::to_vec(&self.persisted_payload()?)
            .map_err(|error| invalid(format!("legal runtime cannot be encoded: {error}")))
            .map(|encoded| encoded.len())
    }

    pub(crate) fn identity_evidence_dependencies(&self) -> IdentityEvidenceDependenciesV1 {
        IdentityEvidenceDependenciesV1::new(
            self.retained_evidence_dependencies
                .iter()
                .cloned()
                .collect(),
        )
    }

    fn proposal_identity_evidence_dependencies(proposal: &LegalProposal) -> BTreeSet<EvidenceRef> {
        let mut dependencies = proposal.evidence.iter().cloned().collect::<BTreeSet<_>>();
        dependencies.extend(
            proposal
                .expected_versions
                .iter()
                .filter(|version| {
                    !matches!(
                        version.established_by,
                        canwu_api::DomainRecordVersionSource::InitialScenario
                    )
                })
                .map(|version| EvidenceRef::DomainRecordVersion(version.clone())),
        );
        dependencies
    }

    fn law_version_identity_evidence_dependencies(
        &self,
        reference: &LegalRecordRef,
    ) -> BTreeSet<EvidenceRef> {
        let mut dependencies = BTreeSet::new();
        let Some(version) = self.law_versions.get(&reference.id) else {
            return dependencies;
        };
        dependencies.extend(version.evidence.iter().cloned());
        if let Some(source) = self.sources.get(&version.source.id) {
            dependencies.extend(source.evidence.iter().cloned());
            match &source.origin {
                Some(LegalOriginRef::Agreement {
                    instrument,
                    ratifications,
                    ..
                }) => {
                    dependencies.insert(EvidenceRef::DomainRecordVersion(instrument.clone()));
                    dependencies.extend(ratifications.iter().cloned());
                }
                Some(LegalOriginRef::Ruling { ruling }) => {
                    if let Some(ruling) = self.rulings.get(&ruling.id) {
                        dependencies.extend(ruling.evidence.iter().cloned());
                        for finding in &ruling.findings {
                            if let Some(finding) = self.findings.get(&finding.id) {
                                dependencies.extend(finding.evidence.iter().cloned());
                            }
                        }
                        for cited_source in &ruling.sources {
                            if let Some(cited_source) = self.sources.get(&cited_source.id) {
                                dependencies.extend(cited_source.evidence.iter().cloned());
                            }
                        }
                        for resolved in &ruling.resolved_versions {
                            if let Some(resolved) = self.law_versions.get(&resolved.id) {
                                dependencies.extend(resolved.evidence.iter().cloned());
                                if let Some(resolved_source) = self.sources.get(&resolved.source.id)
                                {
                                    dependencies.extend(resolved_source.evidence.iter().cloned());
                                }
                            }
                        }
                    }
                }
                Some(LegalOriginRef::Reception {
                    succession,
                    predecessor,
                    ..
                }) => {
                    if let Some(succession) = self
                        .successions
                        .iter()
                        .find(|record| &record.id == succession)
                    {
                        dependencies.extend(succession.evidence.iter().cloned());
                    }
                    if let Some(predecessor) = self.law_versions.get(&predecessor.id) {
                        dependencies.extend(predecessor.evidence.iter().cloned());
                        if let Some(predecessor_source) = self.sources.get(&predecessor.source.id) {
                            dependencies.extend(predecessor_source.evidence.iter().cloned());
                        }
                    }
                }
                None => {}
            }
        }
        dependencies
    }

    fn rebuild_identity_evidence_dependency_counts(&self) -> BTreeMap<EvidenceRef, usize> {
        let mut counts = BTreeMap::new();
        for proposal in self.proposals.values().filter(|proposal| {
            matches!(
                proposal.status,
                ProposalStatus::Draft | ProposalStatus::Submitted | ProposalStatus::Deliberating
            )
        }) {
            increment_dependency_counts(
                &mut counts,
                Self::proposal_identity_evidence_dependencies(proposal),
            );
        }
        for event in self.publicity_events.values() {
            increment_dependency_counts(&mut counts, event.evidence.iter().cloned().collect());
        }
        for procedure_id in &self.open_procedures {
            if let Some(procedure) = self.procedures.get(procedure_id) {
                increment_dependency_counts(
                    &mut counts,
                    procedure.evidence.iter().cloned().collect(),
                );
            }
        }
        for rule in self.rules.values() {
            let live_versions = rule
                .scheduled_versions
                .iter()
                .chain(
                    (!rule.retired)
                        .then_some(&rule.operative_version)
                        .into_iter()
                        .flatten(),
                )
                .chain(rule.latest_adopted_version.iter().filter(|reference| {
                    self.law_versions
                        .get(&reference.id)
                        .is_some_and(|version| !materializes_legal_effect(version))
                }))
                .cloned()
                .collect::<BTreeSet<_>>();
            for version_ref in live_versions {
                increment_dependency_counts(
                    &mut counts,
                    self.law_version_identity_evidence_dependencies(&version_ref),
                );
            }
        }
        counts
    }

    fn add_identity_evidence_component(
        &mut self,
        dependencies: BTreeSet<EvidenceRef>,
    ) -> Result<(), CanwuError> {
        if dependencies.iter().any(|reference| {
            self.retained_evidence_dependency_counts
                .get(reference)
                .is_some_and(|count| *count == usize::MAX)
        }) {
            return Err(invalid("legal evidence dependency count overflowed"));
        }
        for reference in dependencies {
            let count = self
                .retained_evidence_dependency_counts
                .entry(reference.clone())
                .or_default();
            *count += 1;
            self.retained_evidence_dependencies.insert(reference);
        }
        Ok(())
    }

    fn remove_identity_evidence_component(
        &mut self,
        dependencies: BTreeSet<EvidenceRef>,
    ) -> Result<(), CanwuError> {
        if dependencies.iter().any(|reference| {
            self.retained_evidence_dependency_counts
                .get(reference)
                .is_none_or(|count| *count == 0)
        }) {
            return Err(invalid("legal evidence dependency count underflowed"));
        }
        for reference in dependencies {
            let count = self
                .retained_evidence_dependency_counts
                .get_mut(&reference)
                .expect("dependency count was preflighted");
            *count -= 1;
            if *count == 0 {
                self.retained_evidence_dependency_counts.remove(&reference);
                self.retained_evidence_dependencies.remove(&reference);
            }
        }
        Ok(())
    }

    /// Encodes the sole persisted aggregate record owned by this extension.
    ///
    /// Law-local records intentionally remain inside this aggregate; exposing
    /// them as independent Canwu records would require kernel-issued version
    /// provenance that a detached extension cannot manufacture honestly.
    pub fn to_record_drafts(&self) -> Result<Vec<DomainRecordDraft>, CanwuError> {
        Ok(vec![self.to_record_draft()?])
    }

    /// Persist the exact Canwu revision that a later decision enqueue must compare against.
    ///
    /// Callers must settle these ingress records and reload the legal runtime before
    /// calling [`Self::enqueue_pending_decisions`]. This explicit first phase keeps
    /// retries from recapturing a different revision after a crash.
    pub fn prepare_pending_decision_enqueues(
        &self,
        canwu: &mut canwu_api::Canwu,
    ) -> Result<Vec<canwu_api::IngressReceipt>, CanwuError> {
        let expected_revision = canwu
            .revision()
            .checked_add(1)
            .ok_or_else(|| invalid("legal decision expected revision overflowed"))?;
        let mut receipts = Vec::new();
        for sequence in &self.pending_outbox_sequences {
            let item = self
                .outbox
                .get(sequence)
                .ok_or_else(|| invalid("pending legal outbox item is missing"))?;
            if item.enqueue_expected_revision == Some(expected_revision) {
                continue;
            }
            if canwu
                .decision_state()
                .attempt(DecisionRequestId::new(item.create_request_id))
                .is_some()
                || item.refresh_request_id.is_some_and(|request_id| {
                    canwu
                        .decision_state()
                        .attempt(DecisionRequestId::new(request_id))
                        .is_some()
                })
                || canwu
                    .decision_ticket(DecisionTicketId::new(item.ticket_id))
                    .is_some()
            {
                return Err(invalid(
                    "legal outbox cannot reprepare after a core decision outcome exists",
                ));
            }
            if canwu
                .decision_state()
                .controller(&item.decision_controller_id)
                .is_some_and(|controller| match expected_decision_controller(item) {
                    Ok(expected) => controller != &expected,
                    Err(_) => true,
                })
            {
                return Err(invalid(
                    "legal outbox controller binding conflicts with the persisted draft",
                ));
            }
            receipts.push(
                canwu.enqueue_plugin_ingress(canwu_api::PluginIngressRequest::new(
                    crate::PLUGIN_NAME,
                    crate::LAW_OUTBOX_PREPARE_INGRESS,
                    canwu.time(),
                    serde_json::json!({
                        "sequence": sequence,
                        "expected_revision": expected_revision,
                    }),
                ))?,
            );
        }
        Ok(receipts)
    }

    /// Verify that every host-owned record captured while authoring a proposal
    /// is still the current exact version.
    pub fn validate_host_expected_versions(
        &self,
        canwu: &canwu_api::Canwu,
        proposal_id: &str,
    ) -> Result<(), CanwuError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .ok_or_else(|| invalid("legal proposal expected-version guard is missing"))?;
        for expected in &proposal.expected_versions {
            let current = canwu.domain_record(&expected.record).ok_or_else(|| {
                invalid("legal proposal expected host record is no longer available")
            })?;
            if current.version != expected.version
                || !canwu.domain_record_version_evidence_exists(expected)
            {
                return Err(invalid("legal proposal host record compare-and-set failed"));
            }
        }
        Ok(())
    }

    /// Idempotently queues already-prepared legal decision tickets through the public host API.
    ///
    /// This method never rewrites the persisted draft or its expected revision. It
    /// The caller must settle these decisions before calling
    /// [`Self::acknowledge_enqueued_decisions`].
    pub fn enqueue_pending_decisions(
        &self,
        canwu: &mut canwu_api::Canwu,
    ) -> Result<Vec<canwu_api::IngressReceipt>, CanwuError> {
        let sequences = self
            .pending_outbox_sequences
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let prepared = sequences
            .iter()
            .map(|sequence| {
                let item = self
                    .outbox
                    .get(sequence)
                    .ok_or_else(|| invalid("pending legal outbox item is missing"))?;
                let expected_revision = item.enqueue_expected_revision.ok_or_else(|| {
                    invalid("legal outbox expected revision is not durably prepared")
                })?;
                if expected_revision != canwu.revision() {
                    return Err(invalid(
                        "legal outbox preparation is stale; persist a fresh preparation first",
                    ));
                }
                self.validate_host_expected_versions(canwu, &item.proposal.id)?;
                Ok((*sequence, item, expected_revision))
            })
            .collect::<Result<Vec<_>, CanwuError>>()?;
        let mut receipts = Vec::with_capacity(prepared.len());
        let mut queued_controllers = BTreeSet::new();
        for (_sequence, item, expected_revision) in prepared {
            let controller = expected_decision_controller(item)?;
            if item
                .command_subject
                .as_ref()
                .is_some_and(|subject| !canwu.entity_exists(subject))
            {
                return Err(invalid("legal decision command subject is unavailable"));
            }
            let controller_request_id = item
                .refresh_request_id
                .ok_or_else(|| invalid("legal outbox controller request ID is missing"))?;
            match canwu
                .decision_state()
                .controller(&item.decision_controller_id)
            {
                Some(existing) if existing != &controller => {
                    return Err(invalid(
                        "legal decision controller conflicts with the persisted draft",
                    ));
                }
                None if queued_controllers.insert(item.decision_controller_id.clone()) => {
                    canwu.enqueue_decision(
                        item.due_at,
                        item.priority,
                        DecisionIngressRequest::new(
                            DecisionRequestId::new(controller_request_id),
                            expected_revision,
                            DecisionMutation::RegisterController { controller },
                        ),
                    )?;
                }
                Some(_) | None => {}
            }
            let request = DecisionIngressRequest::new(
                DecisionRequestId::new(item.create_request_id),
                expected_revision,
                DecisionMutation::Open {
                    ticket: item.draft.clone(),
                },
            );
            let receipt = canwu.enqueue_decision(item.due_at, item.priority, request)?;
            receipts.push(receipt);
        }
        Ok(receipts)
    }

    /// Bind pending outbox items to accepted core decision outcomes.
    ///
    /// The decision journal and resulting controller/ticket state survive ingress
    /// archival, so a crash after core settlement remains recoverable.
    pub fn acknowledge_enqueued_decisions(
        &self,
        canwu: &mut canwu_api::Canwu,
    ) -> Result<Vec<canwu_api::IngressReceipt>, CanwuError> {
        let mut acknowledgements = Vec::new();
        for sequence in &self.pending_outbox_sequences {
            let item = self
                .outbox
                .get(sequence)
                .ok_or_else(|| invalid("pending legal outbox item is missing"))?;
            let expected_revision = item
                .enqueue_expected_revision
                .ok_or_else(|| invalid("legal outbox expected revision is not durably prepared"))?;
            let controller_request_id = item
                .refresh_request_id
                .ok_or_else(|| invalid("legal outbox controller request ID is missing"))?;
            let controller_attempt = canwu
                .decision_state()
                .attempt(DecisionRequestId::new(controller_request_id));
            if controller_attempt.is_some_and(|attempt| !accepted_without_command(attempt)) {
                return Err(invalid(
                    "legal decision-controller request did not settle as accepted",
                ));
            }
            let open_attempt = canwu
                .decision_state()
                .attempt(DecisionRequestId::new(item.create_request_id))
                .ok_or_else(|| invalid("legal ticket-open request has not settled"))?;
            let controller = canwu
                .decision_state()
                .controller(&item.decision_controller_id)
                .ok_or_else(|| invalid("legal decision controller is missing"))?;
            let ticket = canwu
                .decision_ticket(DecisionTicketId::new(item.ticket_id))
                .ok_or_else(|| invalid("legal decision ticket is missing"))?;
            verify_accepted_outbox_state(
                item,
                expected_revision,
                controller_attempt,
                open_attempt,
                controller,
                ticket,
            )?;
            let outcome_commitment =
                outbox_outcome_commitment(controller_attempt, open_attempt, controller, ticket)?;
            let draft_hash = canonical_hash("canwu.law.decision-draft.v1", &item.draft)?;
            acknowledgements.push(canwu.enqueue_plugin_ingress(
                canwu_api::PluginIngressRequest::new(
                    crate::PLUGIN_NAME,
                    crate::LAW_OUTBOX_ACK_INGRESS,
                    canwu.time(),
                    serde_json::json!({
                        "sequence": sequence,
                        "expected_revision": expected_revision,
                        "controller_request_id": controller_attempt
                            .map(|attempt| attempt.request_id.get()),
                        "create_request_id": item.create_request_id,
                        "ticket_id": item.ticket_id,
                        "draft_hash": draft_hash,
                        "outcome_commitment": outcome_commitment,
                    }),
                ),
            )?);
        }
        Ok(acknowledgements)
    }
}

fn seat_authority(
    plan: &CompiledLawPlan,
    procedure_id: &str,
    seat_id: &str,
) -> (KnowledgeHolderRef, String, String) {
    let authority = plan
        .seat_authority_by_procedure
        .get(procedure_id)
        .and_then(|seats| seats.get(seat_id));
    authority.map_or_else(
        || {
            (
                KnowledgeHolderRef::Entity(EntityRef::Domain(DomainRecordRef::new(
                    PLUGIN_NAMESPACE,
                    "holder",
                    "unassigned",
                ))),
                "unassigned".to_owned(),
                decision_controller_id("unassigned", seat_id),
            )
        },
        |authority| {
            (
                authority.holder.clone().unwrap_or_else(|| {
                    KnowledgeHolderRef::Entity(EntityRef::Domain(DomainRecordRef::new(
                        PLUGIN_NAMESPACE,
                        "holder",
                        "unassigned",
                    )))
                }),
                authority.permission_profile.clone(),
                authority.decision_controller_id.clone(),
            )
        },
    )
}

/// Stable controller identity expected for a compiled institution seat.
#[must_use]
pub fn decision_controller_id(institution: &str, seat: &str) -> String {
    format!(
        "law.i{}.{}.s{}.{}",
        institution.len(),
        institution,
        seat.len(),
        seat
    )
}

#[allow(clippy::too_many_arguments)]
fn decision_ticket_draft(
    proposal: &LegalProposal,
    procedure: &ProcedureInstance,
    stage: &ProcedureStageDefinition,
    seat: &str,
    holder: &KnowledgeHolderRef,
    assigned_controller: &str,
    ticket_id: u64,
    command_request_id: u64,
    due_at: SimTime,
    actor_context: &LegalActorContext,
) -> Result<DecisionTicketDraft, CanwuError> {
    let procedure_ref = procedure_reference(procedure);
    let clause_hash = canonical_hash("canwu.law.proposal-clauses.v1", &proposal.clauses)?;
    let decision_maker = match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    };
    let mut options = Vec::new();
    for ballot in &stage.allowed_ballots {
        let (option_id, label) = ballot_option(*ballot);
        let intent = PendingLegalIntent {
            id: format!(
                "intent:{}:{}:{}:{}:{}",
                procedure.id, procedure.active_stage, procedure.round, seat, option_id
            ),
            command: EvidenceRef::Command(CommandId::new(command_request_id)),
            attempt: None,
            request_id: Some(command_request_id),
            controller: holder.clone(),
            seat: seat.to_owned(),
            proposal: procedure.proposal.clone(),
            procedure: procedure_ref.clone(),
            round: procedure.round,
            stage: procedure.active_stage,
            expected_versions: proposal.expected_versions.clone(),
            selected_option: option_id.to_owned(),
            clause_hash: clause_hash.clone(),
            intended_effective_at: proposal.effective_at,
            admitted_at: due_at,
        };
        let command = Command::Plugin {
            plugin: crate::PLUGIN_NAME.to_owned(),
            command: crate::LAW_COMMAND.to_owned(),
            payload: serde_json::json!({"intent": intent}),
        };
        let mut option = DecisionOption::new(option_id, label);
        option.action = DecisionAction::Command {
            command: serde_json::to_value(command).map_err(|error| {
                invalid(format!("legal decision command cannot be encoded: {error}"))
            })?,
        };
        option.metadata = serde_json::json!({
            "proposal": procedure.proposal,
            "procedure": procedure_ref,
            "stage": stage.id,
            "round": procedure.round,
            "seat": seat,
        });
        options.push(option);
    }
    options.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(DecisionTicketDraft {
        id: DecisionTicketId::new(ticket_id),
        definition: "canwu.law.procedure-seat.v1".to_owned(),
        decision_maker,
        assigned_controller: assigned_controller.to_owned(),
        summary: format!("Decide {} at {}", proposal.id, stage.id),
        context: DecisionContext::new(
            "canwu.law.actor-relative-context.v1",
            serde_json::json!({
                "holder": holder,
                "knowledge_read_cut": actor_context.read_cut,
                "knowledge_record_ids": actor_context.knowledge_record_ids,
                "facts": actor_context.facts,
                "context_hash": actor_context.context_hash,
                "proposal": procedure.proposal,
                "procedure": procedure_ref,
                "stage": stage.id,
                "round": procedure.round,
                "seat": seat,
            }),
        ),
        options,
        deadline: Some(procedure.deadline),
    })
}

fn actor_context_hash(context: &LegalActorContext) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.law.actor-relative-context.v1",
        &(
            &context.holder,
            &context.read_cut,
            &context.knowledge_record_ids,
            &context.facts,
        ),
    )
}

fn validate_proposal_fanout(
    plan: &CompiledLawPlan,
    proposal: &LegalProposal,
) -> Result<(), CanwuError> {
    if proposal.clauses.is_empty()
        || proposal.jurisdictions.is_empty()
        || proposal.clauses.len() > plan.budgets.max_clauses_per_proposal
        || proposal.jurisdictions.len() > plan.budgets.max_jurisdictions_per_proposal
    {
        return Err(invalid(
            "legal proposal clause or jurisdiction budget exceeded",
        ));
    }
    if nested_proposal_items(proposal)
        .is_none_or(|items| items > plan.budgets.max_nested_items_per_record)
    {
        return Err(invalid("legal proposal nested-item budget exceeded"));
    }
    let projections = proposal
        .clauses
        .len()
        .checked_mul(proposal.jurisdictions.len())
        .ok_or_else(|| invalid("legal proposal applicability fan-out overflowed"))?;
    if projections > plan.budgets.max_applicability_entries_per_boundary
        || projections > plan.budgets.max_mutations_per_boundary
    {
        return Err(invalid(
            "legal proposal applicability fan-out budget exceeded",
        ));
    }
    if proposal.cultural_dependencies.iter().any(|dependency| {
        dependency.target.target.is_empty() || !proposal.evidence.contains(&dependency.evidence)
    }) {
        return Err(invalid(
            "legal culture dependency requires a target and admitted evidence",
        ));
    }
    for clause in &proposal.clauses {
        let definition = plan
            .clauses
            .iter()
            .find(|candidate| candidate.id == clause.clause)
            .ok_or_else(|| invalid("legal proposal references an unknown clause"))?;
        let relation_is_complete = match definition.modality {
            NormativeModality::Duty | NormativeModality::Prohibition => {
                !clause.duty_bearers.is_empty()
            }
            NormativeModality::ClaimRight
            | NormativeModality::Power
            | NormativeModality::Liability
            | NormativeModality::Immunity
            | NormativeModality::Disability
            | NormativeModality::Eligibility => {
                !clause.holders.is_empty() && !clause.duty_bearers.is_empty()
            }
            NormativeModality::Liberty | NormativeModality::Status => !clause.holders.is_empty(),
        };
        if !relation_is_complete || clause.subject_matters.is_empty() {
            return Err(invalid(
                "legal clause lacks required parties or a non-empty subject-matter scope",
            ));
        }
        if clause
            .conditions
            .iter()
            .chain(&clause.exceptions)
            .any(|predicate| !plan.predicate_by_id.contains_key(predicate))
        {
            return Err(invalid(
                "legal clause references an unknown compiled predicate",
            ));
        }
        match clause.forum.as_ref() {
            Some(forum_id) => {
                let forum = plan
                    .forum_by_id
                    .get(forum_id)
                    .and_then(|key| plan.forums.get(key.get() as usize))
                    .ok_or_else(|| invalid("legal clause references an unknown forum"))?;
                let forum_is_competent = forum.legal_orders.contains(&proposal.legal_order)
                    && proposal.jurisdictions.contains(&forum.jurisdiction)
                    && clause
                        .subject_matters
                        .iter()
                        .all(|matter| scope_covers(&forum.subject_matters, matter))
                    && clause
                        .standing
                        .iter()
                        .all(|standing| forum.standing_profiles.contains(standing))
                    && clause
                        .remedy_profile
                        .as_ref()
                        .is_none_or(|remedy| forum.remedy_profiles.contains(remedy))
                    && forum.institutions.iter().any(|institution_id| {
                        plan.institutions
                            .iter()
                            .find(|institution| &institution.id == institution_id)
                            .is_some_and(|institution| {
                                institution.competences.iter().any(|competence| {
                                    competence.can_adjudicate
                                        && scope_covers(&competence.forums, forum_id)
                                        && scope_covers(
                                            &competence.legal_orders,
                                            &proposal.legal_order,
                                        )
                                        && scope_covers(
                                            &competence.jurisdictions,
                                            &forum.jurisdiction,
                                        )
                                        && clause.subject_matters.iter().all(|matter| {
                                            scope_covers(&competence.subject_matters, matter)
                                        })
                                })
                            })
                    });
                if !forum_is_competent {
                    return Err(invalid(
                        "legal clause forum, standing, or remedy is outside compiled competence",
                    ));
                }
            }
            None if !clause.standing.is_empty() || clause.remedy_profile.is_some() => {
                return Err(invalid(
                    "legal clause standing or remedy requires an exact compiled forum",
                ));
            }
            None => {}
        }
    }
    Ok(())
}

fn canonicalize_proposal(proposal: &mut LegalProposal) {
    proposal.defects.sort();
    proposal.defects.dedup();
    proposal.jurisdictions.sort();
    proposal.jurisdictions.dedup();
    proposal.subjects.sort();
    proposal.subjects.dedup();
    proposal.cultural_dependencies.sort();
    proposal.cultural_dependencies.dedup();
    if let Some(LegalOriginRef::Agreement {
        parties,
        ratifications,
        ..
    }) = &mut proposal.origin
    {
        parties.sort();
        parties.dedup();
        ratifications.sort();
        ratifications.dedup();
    }
    for clause in &mut proposal.clauses {
        clause.holders.sort();
        clause.holders.dedup();
        clause.duty_bearers.sort();
        clause.duty_bearers.dedup();
        clause.subject_matters.sort();
        clause.subject_matters.dedup();
        clause.territories.sort();
        clause.territories.dedup();
        clause.conditions.sort();
        clause.conditions.dedup();
        clause.exceptions.sort();
        clause.exceptions.dedup();
        clause.standing.sort();
        clause.standing.dedup();
    }
}

fn matching_reception_rule<'a>(
    reception: &'a [ReceptionRule],
    rule_id: &str,
) -> Option<&'a ReceptionRule> {
    reception
        .iter()
        .filter(|rule| !rule.rule_prefix.is_empty() && rule_id.starts_with(&rule.rule_prefix))
        .max_by(|left, right| {
            (left.rule_prefix.len(), left.rule_prefix.as_str())
                .cmp(&(right.rule_prefix.len(), right.rule_prefix.as_str()))
        })
}

fn valid_reception_rule(rule: &ReceptionRule) -> bool {
    !rule.rule_prefix.is_empty()
        && match rule.action {
            ReceptionAction::Transform => rule
                .transform
                .as_ref()
                .is_some_and(|transform| !transform.is_empty()),
            ReceptionAction::Continue | ReceptionAction::Review | ReceptionAction::Displace => {
                rule.transform.is_none()
            }
        }
}

fn canonicalize_succession(succession: &mut LegalOrderSuccession) -> Result<(), CanwuError> {
    succession.predecessors.sort();
    succession.predecessors.dedup();
    succession.successors.sort();
    succession.successors.dedup();
    succession.territorial_scope.sort();
    succession.territorial_scope.dedup();
    succession.personal_scope.sort();
    succession.personal_scope.dedup();
    succession.institutions.sort();
    succession.institutions.dedup();
    succession.liabilities.sort();
    succession.liabilities.dedup();
    succession.archives.sort();
    succession.archives.dedup();
    succession
        .reception
        .sort_by(|left, right| left.rule_prefix.cmp(&right.rule_prefix));
    if succession
        .reception
        .windows(2)
        .any(|pair| pair[0].rule_prefix == pair[1].rule_prefix)
    {
        return Err(invalid("legal succession has duplicate reception prefixes"));
    }
    Ok(())
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn succession_scope_matches(
    succession: &LegalOrderSuccession,
    query: &ApplicabilityQuery,
    subject: Option<&str>,
    actor: Option<&str>,
) -> bool {
    let territory_matches = succession.territorial_scope.is_empty()
        || query.territory.is_some_and(|territory| {
            succession
                .territorial_scope
                .contains(&territory.to_string())
        });
    let personal_matches = succession.personal_scope.is_empty()
        || subject.into_iter().chain(actor).any(|identity| {
            succession
                .personal_scope
                .iter()
                .any(|scope| scope == identity)
        });
    territory_matches && personal_matches
}

fn reachable_jurisdictions(
    plan: &CompiledLawPlan,
    profile: &ApplicabilityProfileDefinition,
    query_jurisdiction: &str,
    query_work: &mut usize,
) -> Result<BTreeSet<String>, CanwuError> {
    let mut visited = BTreeSet::from([query_jurisdiction.to_owned()]);
    let mut pending = vec![query_jurisdiction.to_owned()];
    while let Some(current) = pending.pop() {
        for target in plan
            .jurisdiction_adjacency_by_profile
            .get(&profile.id)
            .and_then(|adjacency| adjacency.get(&current))
            .into_iter()
            .flatten()
        {
            consume_query_work(plan, query_work, 1)?;
            if visited.insert(target.clone()) {
                if visited.len() > profile.max_candidates {
                    return Err(invalid("legal jurisdiction traversal budget exceeded"));
                }
                pending.push(target.clone());
            }
        }
    }
    Ok(visited)
}

fn consume_query_work(
    plan: &CompiledLawPlan,
    query_work: &mut usize,
    amount: usize,
) -> Result<(), CanwuError> {
    *query_work = query_work
        .checked_add(amount)
        .ok_or_else(|| invalid("legal applicability query work overflowed"))?;
    if *query_work > plan.budgets.max_applicability_query_work {
        return Err(invalid("legal applicability query work budget exceeded"));
    }
    Ok(())
}

fn checked_item_sum<const N: usize>(counts: [usize; N]) -> Option<usize> {
    counts.into_iter().try_fold(0_usize, usize::checked_add)
}

fn nested_proposal_items(proposal: &LegalProposal) -> Option<usize> {
    let base = checked_item_sum([
        proposal.jurisdictions.len(),
        proposal.subjects.len(),
        proposal.cultural_dependencies.len(),
        proposal.clauses.len(),
        proposal.defects.len(),
        proposal.admitted_signal_kinds.len(),
        proposal.evidence.len(),
        proposal.expected_versions.len(),
    ])?;
    proposal.clauses.iter().try_fold(base, |total, clause| {
        let clause_items = checked_item_sum([
            clause.holders.len(),
            clause.duty_bearers.len(),
            clause.subject_matters.len(),
            clause.territories.len(),
            clause.conditions.len(),
            clause.exceptions.len(),
            clause.standing.len(),
        ])?;
        total.checked_add(clause_items)
    })
}

fn nested_case_items(case: &LegalCase) -> Option<usize> {
    checked_item_sum([
        case.subject_matters.len(),
        case.parties.len(),
        case.claims.len(),
        case.issues.len(),
        case.remedies.len(),
        case.allegations.len(),
    ])
}

fn nested_ruling_items(ruling: &LegalRulingVersion) -> Option<usize> {
    checked_item_sum([
        ruling.issues.len(),
        ruling.findings.len(),
        ruling.sources.len(),
        ruling.resolved_versions.len(),
        ruling.selected_versions.len(),
        ruling.scope.len(),
        ruling.predecessors.len(),
        ruling.evidence.len(),
    ])
}

fn nested_conflict_items(conflict: &LegalConflict) -> Option<usize> {
    checked_item_sum([
        conflict.versions.len(),
        conflict.governing_versions.len(),
        conflict.displaced_versions.len(),
        conflict.trace.len(),
    ])
}

fn canonicalize_ruling(ruling: &mut LegalRulingVersion) {
    ruling.issues.sort();
    ruling.issues.dedup();
    ruling.findings.sort();
    ruling.findings.dedup();
    ruling.sources.sort();
    ruling.sources.dedup();
    ruling.resolved_versions.sort();
    ruling.resolved_versions.dedup();
    ruling.selected_versions.sort();
    ruling.selected_versions.dedup();
    ruling.scope.sort();
    ruling.scope.dedup();
    ruling.predecessors.sort();
    ruling.predecessors.dedup();
    ruling.evidence.sort();
    ruling.evidence.dedup();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PredicateScopeMatch {
    Applies,
    Excluded,
    Indeterminate,
}

fn predicate_scope_match(
    effect: &NormativeEffect,
    facts: &BTreeMap<String, bool>,
) -> PredicateScopeMatch {
    let mut missing = false;
    for condition in &effect.conditions {
        match facts.get(condition) {
            Some(true) => {}
            Some(false) => return PredicateScopeMatch::Excluded,
            None => missing = true,
        }
    }
    for exception in &effect.exceptions {
        match facts.get(exception) {
            Some(true) => return PredicateScopeMatch::Excluded,
            Some(false) => {}
            None => missing = true,
        }
    }
    if missing {
        PredicateScopeMatch::Indeterminate
    } else {
        PredicateScopeMatch::Applies
    }
}

pub(crate) fn actor_context_from_query_result(
    result: &canwu_api::KnowledgeQueryResult,
) -> Result<LegalActorContext, CanwuError> {
    let mut context = LegalActorContext {
        holder: result.holder.clone(),
        read_cut: result.read_cut.clone(),
        knowledge_record_ids: result.records.iter().map(|record| record.id).collect(),
        facts: serde_json::to_value(&result.records).map_err(|error| {
            invalid(format!(
                "legal holder knowledge cannot be encoded as actor facts: {error}"
            ))
        })?,
        context_hash: String::new(),
    };
    context.context_hash = actor_context_hash(&context)?;
    Ok(context)
}

fn procedure_reference(procedure: &ProcedureInstance) -> LegalRecordRef {
    local_ref("procedure", &procedure.id)
}

fn law_version_reference(version: &LawVersion) -> LegalRecordRef {
    local_ref("law_version", &version.id)
}

struct OutboxIds {
    ticket: u64,
    create: u64,
    refresh: u64,
    resolution: u64,
    command: u64,
}

fn allocate_outbox_ids(plan: &CompiledLawPlan, sequence: u64) -> Result<OutboxIds, CanwuError> {
    let offset = sequence
        .checked_sub(1)
        .ok_or_else(|| invalid("legal outbox sequence zero is invalid"))?;
    let decision_offset = offset
        .checked_mul(3)
        .ok_or_else(|| invalid("legal decision-request ID allocation overflowed"))?;
    if offset >= plan.id_blocks.decision_tickets.capacity
        || offset >= plan.id_blocks.command_requests.capacity
        || decision_offset
            .checked_add(2)
            .is_none_or(|last| last >= plan.id_blocks.decision_requests.capacity)
    {
        return Err(invalid("legal outbox ID block is exhausted"));
    }
    let checked = |start: u64, delta: u64| {
        start
            .checked_add(delta)
            .ok_or_else(|| invalid("legal outbox ID allocation overflowed"))
    };
    Ok(OutboxIds {
        ticket: checked(plan.id_blocks.decision_tickets.start, offset)?,
        create: checked(plan.id_blocks.decision_requests.start, decision_offset)?,
        refresh: checked(plan.id_blocks.decision_requests.start, decision_offset + 1)?,
        resolution: checked(plan.id_blocks.decision_requests.start, decision_offset + 2)?,
        command: checked(plan.id_blocks.command_requests.start, offset)?,
    })
}

fn participation_key(procedure: &str, stage: &str, round: u32, seat: &str) -> String {
    format!("{procedure}\u{1f}{stage}\u{1f}{round:010}\u{1f}{seat}")
}

fn outbox_key(procedure: &str, stage: usize, round: u32, seat: &str) -> String {
    format!("{procedure}\u{1f}{stage:010}\u{1f}{round:010}\u{1f}{seat}")
}

fn procedure_expiry_time(deadline: SimTime) -> Result<SimTime, CanwuError> {
    deadline
        .checked_add(SimDuration::minutes(1))
        .ok_or_else(|| invalid("legal procedure expiry time overflowed"))
}

fn ballot_for_option(option: &str) -> Ballot {
    match option {
        "against" => Ballot::Against,
        "abstain" => Ballot::Abstain,
        "veto" => Ballot::Veto,
        _ => Ballot::For,
    }
}

const fn ballot_option(ballot: Ballot) -> (&'static str, &'static str) {
    match ballot {
        Ballot::For => ("for", "Vote for"),
        Ballot::Against => ("against", "Vote against"),
        Ballot::Abstain => ("abstain", "Abstain"),
        Ballot::Veto => ("veto", "Veto"),
    }
}

fn institutional_competence_covers(
    plan: &CompiledLawPlan,
    profile: &CompiledSourceProfile,
    proposal: &LegalProposal,
) -> bool {
    let Some(procedure) = profile.procedure.as_ref() else {
        return true;
    };
    let Some(authorities) = plan.seat_authority_by_procedure.get(procedure) else {
        return false;
    };
    let institutions = authorities
        .values()
        .map(|authority| authority.institution.as_str())
        .collect::<BTreeSet<_>>();
    !institutions.is_empty()
        && institutions.into_iter().all(|institution_id| {
            plan.institutions
                .iter()
                .find(|institution| institution.id == institution_id)
                .is_some_and(|institution| {
                    institution.competences.iter().any(|competence| {
                        competence.source_modes.contains(&profile.mode)
                            && competence.operations.contains(&proposal.operation)
                            && scope_covers(&competence.procedures, procedure)
                            && scope_covers(&competence.legal_orders, &proposal.legal_order)
                            && proposal
                                .jurisdictions
                                .iter()
                                .all(|id| scope_covers(&competence.jurisdictions, id))
                            && proposal.clauses.iter().all(|clause| {
                                clause
                                    .subject_matters
                                    .iter()
                                    .all(|matter| scope_covers(&competence.subject_matters, matter))
                            })
                    })
                })
        })
}

fn scope_covers(scope: &[String], value: &str) -> bool {
    scope.iter().any(|item| item == "*" || item == value)
}

pub(crate) fn expected_decision_controller(
    item: &LegalDecisionOutboxItem,
) -> Result<DecisionControllerBinding, CanwuError> {
    let authority = match &item.controller {
        KnowledgeHolderRef::Person(actor) => DecisionAuthority::Actor { actor: *actor },
        KnowledgeHolderRef::Entity(institution) => DecisionAuthority::Institution {
            institution: institution.clone(),
            responsible_actor: None,
        },
    };
    let mut controller = DecisionControllerBinding::new(
        item.decision_controller_id.clone(),
        DecisionPolicyIdentity::new(DecisionPolicyKind::Human, "canwu-law-human-seat", "1"),
        authority,
    )
    .with_seat(&item.seat, &item.permission_profile_id);
    if let Some(subject) = &item.command_subject {
        controller = controller.with_command_subject(subject.clone());
    }
    Ok(controller)
}

pub(crate) fn verify_accepted_outbox_state(
    item: &LegalDecisionOutboxItem,
    expected_revision: u64,
    controller_attempt: Option<&DecisionAttemptRecord>,
    open_attempt: &DecisionAttemptRecord,
    controller: &DecisionControllerBinding,
    ticket: &DecisionTicket,
) -> Result<(), CanwuError> {
    let controller_request_id = item
        .refresh_request_id
        .ok_or_else(|| invalid("legal outbox controller request ID is missing"))?;
    let expected_controller = expected_decision_controller(item)?;
    let expected_controller_request = DecisionIngressRequest::new(
        DecisionRequestId::new(controller_request_id),
        expected_revision,
        DecisionMutation::RegisterController {
            controller: expected_controller.clone(),
        },
    );
    let expected_controller_commitment = canonical_hash(
        DECISION_REQUEST_COMMITMENT_DOMAIN,
        &expected_controller_request,
    )?;
    let expected_open_request = DecisionIngressRequest::new(
        DecisionRequestId::new(item.create_request_id),
        expected_revision,
        DecisionMutation::Open {
            ticket: item.draft.clone(),
        },
    );
    let expected_open_commitment =
        canonical_hash(DECISION_REQUEST_COMMITMENT_DOMAIN, &expected_open_request)?;
    if controller_attempt.is_some_and(|attempt| {
        attempt.request_id.get() != controller_request_id
            || attempt.expected_revision != expected_revision
            || attempt.request_commitment != expected_controller_commitment
            || !accepted_without_command(attempt)
    }) {
        return Err(invalid(
            "legal outbox controller attempt does not match its persisted request",
        ));
    }
    if open_attempt.request_id.get() != item.create_request_id
        || open_attempt.expected_revision != expected_revision
        || open_attempt.request_commitment != expected_open_commitment
        || !accepted_without_command(open_attempt)
    {
        return Err(invalid(
            "legal outbox ticket-open attempt does not match its persisted request",
        ));
    }
    if controller != &expected_controller {
        return Err(invalid(
            "legal outbox settled controller does not match its persisted binding",
        ));
    }
    let ticket_mismatch = if ticket.id.get() != item.ticket_id {
        Some("identity")
    } else if ticket.definition != item.draft.definition {
        Some("definition")
    } else if ticket.decision_maker != item.draft.decision_maker {
        Some("decision maker")
    } else if ticket.assigned_controller != item.draft.assigned_controller {
        Some("assigned controller")
    } else if ticket.summary != item.draft.summary {
        Some("summary")
    } else if ticket.context != item.draft.context {
        Some("context")
    } else if ticket.options != item.draft.options {
        Some("options")
    } else if ticket.deadline != item.draft.deadline {
        Some("deadline")
    } else if ticket.opened_at != open_attempt.at {
        Some("opened time")
    } else {
        None
    };
    if let Some(field) = ticket_mismatch {
        return Err(invalid(format!(
            "legal outbox settled ticket {field} does not match its persisted draft"
        )));
    }
    Ok(())
}

pub(crate) fn outbox_outcome_commitment(
    controller_attempt: Option<&DecisionAttemptRecord>,
    open_attempt: &DecisionAttemptRecord,
    controller: &DecisionControllerBinding,
    ticket: &DecisionTicket,
) -> Result<String, CanwuError> {
    canonical_hash(
        "canwu.law.accepted-outbox.v1",
        &(
            controller_attempt,
            open_attempt,
            controller,
            ticket.id,
            &ticket.definition,
            &ticket.decision_maker,
            &ticket.assigned_controller,
            &ticket.summary,
            &ticket.context,
            &ticket.options,
            ticket.opened_at,
            ticket.deadline,
        ),
    )
}

fn accepted_without_command(attempt: &DecisionAttemptRecord) -> bool {
    matches!(
        attempt.outcome,
        DecisionAttemptOutcome::Accepted {
            trace_id: None,
            command_request_id: None
        }
    )
}

const fn disposition_for_operation(operation: LawOperation) -> OperativeDisposition {
    match operation {
        LawOperation::Suspend => OperativeDisposition::Suspended,
        LawOperation::Displace => OperativeDisposition::Displaced,
        LawOperation::Annul => OperativeDisposition::Annulled,
        LawOperation::Repeal => OperativeDisposition::Repealed,
        LawOperation::Expire => OperativeDisposition::Expired,
        LawOperation::Establish
        | LawOperation::Recognize
        | LawOperation::Receive
        | LawOperation::Amend
        | LawOperation::Resume => OperativeDisposition::Operative,
    }
}

const fn legal_version_disposition(proposal: &LegalProposal) -> OperativeDisposition {
    if matches!(proposal.validity, OperativeDisposition::Operative) {
        disposition_for_operation(proposal.operation)
    } else {
        proposal.validity
    }
}

const fn materializes_legal_effect(version: &LawVersion) -> bool {
    !matches!(
        version.disposition,
        OperativeDisposition::Claimed
            | OperativeDisposition::Purported
            | OperativeDisposition::Contested
    )
}

const fn retires_rule(operation: LawOperation) -> bool {
    matches!(
        operation,
        LawOperation::Annul | LawOperation::Repeal | LawOperation::Expire
    )
}

fn local_ref(kind: &str, id: &str) -> LegalRecordRef {
    LegalRecordRef {
        kind: kind.to_owned(),
        id: id.to_owned(),
    }
}

fn increment_dependency_counts(
    counts: &mut BTreeMap<EvidenceRef, usize>,
    dependencies: BTreeSet<EvidenceRef>,
) {
    for reference in dependencies {
        *counts.entry(reference).or_default() += 1;
    }
}

fn legal_claim_fields_match(
    competence: LegalCompetenceDisposition,
    defects: &[String],
    validity: OperativeDisposition,
) -> bool {
    let allowed_validity = matches!(
        validity,
        OperativeDisposition::Claimed
            | OperativeDisposition::Purported
            | OperativeDisposition::Operative
            | OperativeDisposition::Contested
    );
    allowed_validity
        && match competence {
            LegalCompetenceDisposition::Confirmed => {
                defects.is_empty() || validity != OperativeDisposition::Operative
            }
            LegalCompetenceDisposition::Purported => {
                !defects.is_empty() && validity == OperativeDisposition::Purported
            }
            LegalCompetenceDisposition::Contested => {
                !defects.is_empty() && validity == OperativeDisposition::Contested
            }
        }
}

mod evidence_dependency_counts_serde {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(
        counts: &BTreeMap<EvidenceRef, usize>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        counts.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<EvidenceRef, usize>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<(EvidenceRef, usize)>::deserialize(deserializer)?;
        let mut counts = BTreeMap::new();
        for (reference, count) in entries {
            if count == 0 || counts.insert(reference, count).is_some() {
                return Err(serde::de::Error::custom(
                    "evidence dependency counts must be positive and unique",
                ));
            }
        }
        Ok(counts)
    }
}

fn validate_runtime_text_budget<T: Serialize + ?Sized>(
    value: &T,
    max_text_bytes: usize,
) -> Result<(), CanwuError> {
    let value = serde_json::to_value(value)
        .map_err(|error| invalid(format!("legal runtime input cannot be encoded: {error}")))?;
    fn validate(value: &serde_json::Value, max: usize) -> bool {
        match value {
            serde_json::Value::String(text) => text.len() <= max,
            serde_json::Value::Array(values) => values.iter().all(|value| validate(value, max)),
            serde_json::Value::Object(values) => values
                .iter()
                .all(|(key, value)| key.len() <= max && validate(value, max)),
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
                true
            }
        }
    }
    if !validate(&value, max_text_bytes) {
        return Err(invalid("legal runtime text budget exhausted"));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(canwu_api::ErrorCode::InvalidDomainRecord, message)
}
