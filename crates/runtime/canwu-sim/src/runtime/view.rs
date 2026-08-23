use super::{
    ActorKnowledge, Army, ArmyId, BoundaryId, BoundaryKnowledgeChange, CanwuError, CommandId,
    CommandRecord, DomainRecord, DomainRecordKind, DomainRecordRef, DomainRecordType,
    DomainRecordVersionRef, EntityRef, ErrorCode, EventId, EvidenceRef, Government, GovernmentId,
    HashSet, IngressId, IngressPayload, IngressRecord, KnowledgeHolderRef, KnowledgeQuery,
    KnowledgeRecord, KnowledgeRecordId, Person, PersonId, PluginComponentKey,
    PluginComponentRecord, RandomOperationTarget, RandomStreamKey, RefCell, ReservationAllocation,
    ReservationRef, Route, RouteId, RuntimeCurrentState, RuntimeEvidence, RuntimeState, SimEvent,
    SimTime, StateKey, Territory, TerritoryId, TypedDomainRecordRef, Value, component_key, random,
    records, validation,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) enum SimulationViewState<'a> {
    Runtime(&'a RuntimeState),
    Boundary {
        current: &'a RuntimeCurrentState,
        now: SimTime,
        runtime: &'a RuntimeState,
    },
}

impl SimulationViewState<'_> {
    const fn current(&self) -> &RuntimeCurrentState {
        match self {
            Self::Runtime(state) => &state.current,
            Self::Boundary { current, .. } => current,
        }
    }

    const fn now(&self) -> SimTime {
        match self {
            Self::Runtime(state) => state.scheduler.now,
            Self::Boundary { now, .. } => *now,
        }
    }

    const fn evidence(&self) -> &RuntimeEvidence {
        match self {
            Self::Runtime(state) => &state.evidence,
            Self::Boundary { runtime, .. } => &runtime.evidence,
        }
    }

    const fn runtime(&self) -> &RuntimeState {
        match self {
            Self::Runtime(state) | Self::Boundary { runtime: state, .. } => state,
        }
    }
}

pub struct SimulationView<'a> {
    pub(super) state: SimulationViewState<'a>,
    pub(super) state_owners: &'a BTreeMap<StateKey, String>,
    pub(super) reader: Option<&'a str>,
    pub(super) allowed_reads: Option<&'a [StateKey]>,
    pub(super) allowed_ingress: Option<&'a HashSet<IngressId>>,
    pub(super) ingress_plugin: Option<&'a str>,
    pub(super) component_overlay: Option<&'a BTreeMap<PluginComponentKey, PluginComponentRecord>>,
    pub(super) proposed_components: Option<&'a BTreeMap<PluginComponentKey, PluginComponentRecord>>,
    pub(super) record_overlay: Option<&'a BTreeMap<DomainRecordRef, DomainRecord>>,
    pub(super) proposed_records: Option<&'a BTreeMap<DomainRecordRef, DomainRecord>>,
    pub(super) boundary_id: Option<BoundaryId>,
    pub(super) proposal_evidence: Option<&'a BTreeSet<EvidenceRef>>,
    pub(super) knowledge_overlay:
        Option<&'a BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>>,
    pub(super) allocations: Option<&'a BTreeMap<ReservationRef, ReservationAllocation>>,
    pub(super) allowed_reservations: Option<&'a [ReservationRef]>,
    pub(super) random_session: Option<RefCell<random::RandomSession>>,
}

impl SimulationView<'_> {
    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.state.now()
    }

    pub fn army(&self, id: ArmyId) -> Result<Option<&Army>, CanwuError> {
        self.require_read(&StateKey::core_armies())?;
        Ok(self.state.current().armies.get(&id))
    }

    pub fn person(&self, id: PersonId) -> Result<Option<&Person>, CanwuError> {
        self.require_read(&StateKey::core_people())?;
        Ok(self.state.current().people.get(&id))
    }

    pub fn government(&self, id: GovernmentId) -> Result<Option<&Government>, CanwuError> {
        self.require_read(&StateKey::core_governments())?;
        Ok(self.state.current().governments.get(&id))
    }

    pub fn territory(&self, id: TerritoryId) -> Result<Option<&Territory>, CanwuError> {
        self.require_read(&StateKey::core_territories())?;
        Ok(self.state.current().territories.get(&id))
    }

    pub fn route(&self, id: RouteId) -> Result<Option<&Route>, CanwuError> {
        self.require_read(&StateKey::core_routes())?;
        Ok(self.state.current().routes.get(&id))
    }

    pub fn actor_knowledge(&self, actor: PersonId) -> Result<Option<&ActorKnowledge>, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        Ok(self.state.current().knowledge.for_actor(actor))
    }

    /// Queries holder-relative records for an omniscient plugin system.
    ///
    /// This enforces the declared `canwu.core.knowledge` read and returns an
    /// owned projection. It is not an actor-facing authorization API.
    pub fn knowledge_records(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
    ) -> Result<canwu_knowledge::KnowledgeQueryResult, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        let result = if let Some(overlay) = self.knowledge_overlay {
            self.state.current().knowledge.query_with_overlay(
                holder,
                query,
                self.boundary_id,
                overlay,
            )
        } else {
            self.state
                .current()
                .knowledge
                .query_current(holder, query, self.boundary_id)
        };
        result.map_err(|error| match error {
            canwu_knowledge::KnowledgeQueryError::ReadCutUnavailable => CanwuError::new(
                ErrorCode::KnowledgeReadCutUnavailable,
                "knowledge cursor read cut is no longer available",
            ),
            canwu_knowledge::KnowledgeQueryError::InvalidLimit => CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "knowledge query page size is outside the supported range",
            ),
            canwu_knowledge::KnowledgeQueryError::InvalidCursor
            | canwu_knowledge::KnowledgeQueryError::InvalidLedger
            | canwu_knowledge::KnowledgeQueryError::Encoding => CanwuError::new(
                ErrorCode::InvalidKnowledgeRecord,
                "knowledge query, cursor, or ledger is invalid",
            ),
        })
    }

    /// Resolves an exact command ID from the retained runtime journal in O(1).
    ///
    /// An archived command remains valid identity evidence, but its payload is
    /// no longer available through this view: lookup returns
    /// [`ErrorCode::EvidenceContentUnavailable`]. `None` means the ID has
    /// neither retained content nor a committed archive receipt.
    pub fn command(&self, id: CommandId) -> Result<Option<&CommandRecord>, CanwuError> {
        self.require_read(&StateKey::core_commands())?;
        let retained = self.state.evidence().retained_command(id);
        if retained.is_none()
            && self
                .state
                .evidence()
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Command(id))
        {
            return Err(CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "command identity is archived; payload inspection requires an archive provider",
            ));
        }
        Ok(retained)
    }

    /// Resolves an exact event ID from the retained runtime journal in O(1).
    ///
    /// An archived event remains valid identity evidence, but its payload is
    /// no longer available through this view: lookup returns
    /// [`ErrorCode::EvidenceContentUnavailable`]. `None` means the ID has
    /// neither retained content nor a committed archive receipt.
    pub fn event(&self, id: EventId) -> Result<Option<&SimEvent>, CanwuError> {
        self.require_read(&StateKey::core_events())?;
        let retained = self.state.evidence().retained_event(id);
        if retained.is_none()
            && self
                .state
                .evidence()
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Event(id))
        {
            return Err(CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "event identity is archived; payload inspection requires an archive provider",
            ));
        }
        Ok(retained)
    }

    pub fn ingress(&self, id: IngressId) -> Result<Option<&IngressRecord>, CanwuError> {
        self.require_read(&StateKey::core_ingress())?;
        if self
            .allowed_ingress
            .is_none_or(|allowed| !allowed.contains(&id))
        {
            return Ok(None);
        }
        let record = self.state.evidence().retained_ingress(id);
        if record.is_none()
            && self
                .state
                .evidence()
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Ingress(id))
        {
            return Err(CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "ingress identity is archived; payload inspection requires an archive provider",
            ));
        }
        if let (Some(owner), Some(record)) = (self.ingress_plugin, record)
            && !matches!(
                &record.payload,
                IngressPayload::Plugin { plugin, .. } if plugin == owner
            )
        {
            return Ok(None);
        }
        Ok(record)
    }

    pub fn domain_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.kind))?;
        Ok(self
            .record_overlay
            .and_then(|overlay| overlay.get(reference))
            .or_else(|| self.state.current().domain_records.get(reference)))
    }

    pub fn typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.domain_record(reference.as_untyped())
    }

    pub fn proposed_domain_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.kind))?;
        Ok(self
            .proposed_records
            .and_then(|records| records.get(reference)))
    }

    pub fn proposed_typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.proposed_domain_record(reference.as_untyped())
    }

    /// Returns the exact evidence reference assigned to a domain-record
    /// version proposed earlier in the current boundary.
    pub fn proposed_domain_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.kind))?;
        Ok(self.proposal_evidence.and_then(|evidence| {
            evidence.iter().find_map(|item| match item {
                EvidenceRef::DomainRecordVersion(version) if version.record == *reference => {
                    Some(version.clone())
                }
                _ => None,
            })
        }))
    }

    /// Returns whether an exact domain-record version reference is valid at
    /// this proposal-visible cut.
    ///
    /// This validates both the record identity/version and its establishment
    /// source. Earlier same-boundary proposals are considered before retained
    /// or archived runtime evidence.
    pub fn domain_record_version_evidence_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<bool, CanwuError> {
        self.require_read(&records::record_state_key(&reference.record.kind))?;
        if let Some(proposed) = self.proposed_domain_record_version(&reference.record)? {
            return Ok(proposed == *reference);
        }
        Ok(!matches!(
            validation::resolve_evidence_reference(
                &validation::RuntimeValidationContext::new(self.state.runtime()),
                &EvidenceRef::DomainRecordVersion(reference.clone()),
            ),
            validation::EvidenceAvailability::Missing
        ))
    }

    /// Returns a bounded, deterministic projection of records of one kind.
    ///
    /// Same-boundary overlays take precedence over current state. Records are
    /// ordered by their canonical reference, so result order is replay stable.
    pub fn domain_records_of_kind(
        &self,
        kind: &DomainRecordKind,
        limit: usize,
    ) -> Result<Vec<DomainRecord>, CanwuError> {
        self.domain_records_of_kind_after(kind, None, limit)
    }

    /// Returns one bounded deterministic page of records after a canonical
    /// record-reference cursor.
    ///
    /// The cursor is exclusive and must name the same kind. This keeps plugin
    /// scans bounded without imposing a 10,000-record lifetime ceiling on a
    /// domain kind. Same-boundary overlays retain the same precedence as
    /// [`Self::domain_records_of_kind`].
    pub fn domain_records_of_kind_after(
        &self,
        kind: &DomainRecordKind,
        after: Option<&DomainRecordRef>,
        limit: usize,
    ) -> Result<Vec<DomainRecord>, CanwuError> {
        const MAX_DOMAIN_RECORD_QUERY_LIMIT: usize = 10_000;
        self.require_read(&records::record_state_key(kind))?;
        if limit == 0 || limit > MAX_DOMAIN_RECORD_QUERY_LIMIT {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                format!(
                    "domain-record query limit must be between 1 and {MAX_DOMAIN_RECORD_QUERY_LIMIT}"
                ),
            ));
        }
        if after.is_some_and(|cursor| cursor.kind != *kind) {
            return Err(CanwuError::new(
                ErrorCode::InvalidPayload,
                "domain-record page cursor has the wrong kind",
            ));
        }

        let mut records = BTreeMap::new();
        for (reference, record) in &self.state.current().domain_records {
            if reference.kind == *kind {
                records.insert(reference.clone(), record.clone());
            }
        }
        for overlay in [self.record_overlay, self.proposed_records]
            .into_iter()
            .flatten()
        {
            for (reference, record) in overlay {
                if reference.kind == *kind {
                    records.insert(reference.clone(), record.clone());
                }
            }
        }
        Ok(records
            .into_iter()
            .filter(|(reference, _)| after.is_none_or(|cursor| reference > cursor))
            .map(|(_, record)| record)
            .take(limit)
            .collect())
    }

    /// Finds committed knowledge changes produced with an exact correlation.
    ///
    /// This supports next-boundary operation finalization without granting a
    /// plugin unrestricted access to unrelated knowledge payloads.
    pub fn knowledge_changes_by_correlation(
        &self,
        plugin: &str,
        producer_correlation: &str,
    ) -> Result<Vec<BoundaryKnowledgeChange>, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        Ok(self
            .state
            .evidence()
            .boundaries
            .iter()
            .flat_map(|boundary| &boundary.knowledge_changes)
            .filter(|change| {
                change.plugin == plugin
                    && change.producer_correlation.as_deref() == Some(producer_correlation)
            })
            .cloned()
            .collect())
    }

    /// Finds committed knowledge changes whose producer correlation begins
    /// with a deterministic operation prefix.
    ///
    /// The prefix remains plugin-scoped. This is intended for bounded
    /// multi-holder operation finalization where every holder batch must keep
    /// a unique full correlation value.
    pub fn knowledge_changes_by_correlation_prefix(
        &self,
        plugin: &str,
        producer_correlation_prefix: &str,
    ) -> Result<Vec<BoundaryKnowledgeChange>, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        Ok(self
            .state
            .evidence()
            .boundaries
            .iter()
            .flat_map(|boundary| &boundary.knowledge_changes)
            .filter(|change| {
                change.plugin == plugin
                    && change
                        .producer_correlation
                        .as_deref()
                        .is_some_and(|value| value.starts_with(producer_correlation_prefix))
            })
            .cloned()
            .collect())
    }

    pub fn reservation(
        &self,
        reservation: &ReservationRef,
    ) -> Result<Option<&ReservationAllocation>, CanwuError> {
        let reader = self.reader.unwrap_or("unscoped caller");
        if self
            .allowed_reservations
            .is_none_or(|allowed| !allowed.contains(reservation))
        {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "system {reader} did not declare reservation read {}.{}.{}",
                    reservation.plugin, reservation.system, reservation.request
                ),
            ));
        }
        Ok(self.allocations.and_then(|values| values.get(reservation)))
    }

    pub fn random_range(
        &self,
        stream: &RandomStreamKey,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<u64, CanwuError> {
        let Some(session) = &self.random_session else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredRandomStream,
                format!(
                    "system {} has no declared random streams",
                    self.reader.unwrap_or("unscoped caller")
                ),
            ));
        };
        session.borrow_mut().range(stream, upper_exclusive, purpose)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn random_range_for_operation(
        &self,
        stream: &RandomStreamKey,
        evidence: EvidenceRef,
        operation_kind: &str,
        application_operation_id: &str,
        target: RandomOperationTarget,
        draw_slot: u32,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<u64, CanwuError> {
        let available = self
            .proposal_evidence
            .is_some_and(|values| values.contains(&evidence))
            || validation::resolve_evidence_reference(
                &validation::RuntimeValidationContext::new(self.state.runtime()),
                &evidence,
            ) == validation::EvidenceAvailability::Retained;
        if !available {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomOperationEvidence,
                "operation-keyed random draw references unavailable evidence",
            ));
        }
        let Some(session) = &self.random_session else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredRandomStream,
                format!(
                    "system {} has no declared random streams",
                    self.reader.unwrap_or("unscoped caller")
                ),
            ));
        };
        session.borrow_mut().range_for_operation(
            stream,
            evidence,
            operation_kind,
            application_operation_id,
            target,
            draw_slot,
            upper_exclusive,
            purpose,
        )
    }

    pub fn component(
        &self,
        state: &StateKey,
        entity: &EntityRef,
        component: &str,
    ) -> Result<Option<&Value>, CanwuError> {
        self.require_read(state)?;
        let Some(owner) = self.state_owners.get(state) else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "state {}.{} has no registered owner",
                    state.namespace, state.name
                ),
            ));
        };
        let key = component_key(owner, state, entity, component);
        Ok(self
            .component_overlay
            .and_then(|overlay| overlay.get(&key))
            .or_else(|| self.state.current().plugin_components.get(&key))
            .map(|record| &record.value))
    }

    pub fn proposed_component(
        &self,
        state: &StateKey,
        entity: &EntityRef,
        component: &str,
    ) -> Result<Option<&Value>, CanwuError> {
        self.require_read(state)?;
        let Some(owner) = self.state_owners.get(state) else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "state {}.{} has no registered owner",
                    state.namespace, state.name
                ),
            ));
        };
        let key = component_key(owner, state, entity, component);
        Ok(self
            .proposed_components
            .and_then(|proposals| proposals.get(&key))
            .map(|record| &record.value))
    }

    fn require_read(&self, state: &StateKey) -> Result<(), CanwuError> {
        if self
            .allowed_reads
            .is_some_and(|reads| !reads.contains(state))
        {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "{} did not declare read access to {}.{}",
                    self.reader.unwrap_or("internal system"),
                    state.namespace,
                    state.name
                ),
            ));
        }
        Ok(())
    }

    pub(super) fn finish_random_session(self) -> Option<random::RandomExecution> {
        self.random_session
            .map(RefCell::into_inner)
            .map(random::RandomSession::finish)
    }
}
