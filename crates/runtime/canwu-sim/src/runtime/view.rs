use super::{
    ActorKnowledge, Army, ArmyId, BoundaryId, BoundaryKnowledgeChange, CanwuError, CauseRef,
    CommandId, CommandRecord, DecisionAttemptRecord, DecisionControllerBinding, DecisionRequestId,
    DecisionTicket, DecisionTicketId, DomainRecord, DomainRecordKind, DomainRecordRef,
    DomainRecordType, DomainRecordVersionRef, EntityRef, ErrorCode, EventId, EvidenceRef,
    Government, GovernmentId, HashSet, IngressId, IngressPayload, IngressQueueKey, IngressRecord,
    KnowledgeHolderRef, KnowledgeQuery, KnowledgeRecord, KnowledgeRecordId, Person, PersonId,
    PluginComponentKey, PluginComponentRecord, RandomOperationTarget, RandomStreamKey, RefCell,
    ReservationAllocation, ReservationRef, Route, RouteId, RuntimeCurrentState, RuntimeEvidence,
    RuntimeState, SimEvent, SimTime, StateKey, Territory, TerritoryId, TypedDomainRecordRef, Value,
    component_key, domain_record_candidates, random, records, retained_domain_record_version,
    validate_domain_record_page_request, validation,
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
    pub(super) plugin_archive_provider: &'a dyn super::PluginArchiveObjectProvider,
}

impl SimulationView<'_> {
    /// Loads a package-owned cold object through the host provider attached to
    /// this runtime. Package code remains responsible for authenticating the
    /// bytes against its committed archive root before using them.
    pub fn plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.plugin_archive_provider
            .load_plugin_archive_object(namespace, object_id)
    }

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

    /// Counts records in a knowledge namespace at the current proposal-visible cut.
    pub fn knowledge_record_count_in_namespace(
        &self,
        namespace: &str,
    ) -> Result<usize, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        let settled = self
            .state
            .current()
            .knowledge
            .record_count_in_namespace(namespace);
        let proposed = self.knowledge_overlay.map_or(0, |overlay| {
            overlay
                .values()
                .flat_map(BTreeMap::values)
                .filter(|record| record.schema.kind.namespace == namespace)
                .count()
        });
        settled.checked_add(proposed).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "knowledge namespace record count overflowed",
            )
        })
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

    /// Matches retained, plugin-generated ingress provenance without exposing its payload.
    ///
    /// Durable evidence may cite ingress admitted at an earlier boundary, but a
    /// generated record still waiting in the scheduler is not yet admissible.
    /// Format-7 archive receipts retain a Merkle-bound compact producer proof,
    /// so archived payload bytes do not need to return to the hot path.
    pub fn plugin_ingress_matches(
        &self,
        id: IngressId,
        plugin: &str,
        packet_type: &str,
    ) -> Result<bool, CanwuError> {
        self.require_read(&StateKey::core_ingress())?;
        let record = self.state.evidence().retained_ingress(id);
        if record.is_none()
            && let Some(receipt) = self
                .state
                .evidence()
                .archived_evidence_receipts
                .get(&EvidenceRef::Ingress(id))
        {
            if self
                .state
                .runtime()
                .scheduler
                .pending_ingress
                .iter()
                .any(|key| key.id == id)
            {
                return Ok(false);
            }
            return Ok(receipt
                .plugin_ingress_provenance
                .as_ref()
                .is_some_and(|provenance| {
                    provenance.plugin == plugin && provenance.packet_type == packet_type
                }));
        }
        let Some(record) = record else {
            return Ok(false);
        };
        if self
            .state
            .runtime()
            .scheduler
            .pending_ingress
            .contains(&IngressQueueKey::from_record(record))
        {
            return Ok(false);
        }
        if !matches!(
            &record.payload,
            IngressPayload::Plugin {
                plugin: actual_plugin,
                packet_type: actual_packet_type,
                ..
            } if actual_plugin == plugin && actual_packet_type == packet_type
        ) {
            return Ok(false);
        }
        let Some(CauseRef::Boundary(boundary_id)) = record.cause.as_ref() else {
            return Ok(false);
        };
        let boundary = self.state.evidence().retained_boundary(*boundary_id);
        if boundary.is_none()
            && self
                .state
                .evidence()
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Boundary(*boundary_id))
        {
            return Err(CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "ingress producer boundary is archived; provenance inspection requires an archive provider",
            ));
        }
        Ok(boundary.is_some_and(|boundary| {
            boundary
                .generated_ingress
                .iter()
                .any(|generation| generation.ingress == id && generation.plugin == plugin)
        }))
    }

    /// Matches a retained plugin ingress to an exact provider payload and
    /// delivery time. Payload inspection is deliberately limited to retained
    /// records: archived receipts prove producer identity, but cannot safely
    /// be reused to authorize a different legal proposal without the original
    /// bytes.
    pub fn plugin_ingress_payload_matches(
        &self,
        id: IngressId,
        plugin: &str,
        packet_type: &str,
        occurred_at: SimTime,
        expected_payload: &Value,
    ) -> Result<bool, CanwuError> {
        self.require_read(&StateKey::core_ingress())?;
        let Some(record) = self.state.evidence().retained_ingress(id) else {
            if self
                .state
                .evidence()
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Ingress(id))
            {
                return Err(CanwuError::new(
                    ErrorCode::EvidenceContentUnavailable,
                    "provider ingress payload is archived; exact legal signal binding requires retained content",
                ));
            }
            return Ok(false);
        };
        if self
            .state
            .runtime()
            .scheduler
            .pending_ingress
            .contains(&IngressQueueKey::from_record(record))
        {
            return Ok(false);
        }
        let IngressPayload::Plugin {
            plugin: actual_plugin,
            packet_type: actual_packet_type,
            payload,
            ..
        } = &record.payload
        else {
            return Ok(false);
        };
        if actual_plugin != plugin
            || actual_packet_type != packet_type
            || record.due_at != occurred_at
            || payload != expected_payload
        {
            return Ok(false);
        }
        let Some(CauseRef::Boundary(boundary_id)) = record.cause.as_ref() else {
            return Ok(false);
        };
        let boundary = self.state.evidence().retained_boundary(*boundary_id);
        if boundary.is_none()
            && self
                .state
                .evidence()
                .archived_evidence_receipts
                .contains_key(&EvidenceRef::Boundary(*boundary_id))
        {
            return Err(CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "provider ingress producer boundary is archived; exact legal signal binding requires retained content",
            ));
        }
        Ok(boundary.is_some_and(|boundary| {
            boundary
                .generated_ingress
                .iter()
                .any(|generation| generation.ingress == id && generation.plugin == plugin)
        }))
    }

    /// Returns the retained outcome for one exact decision request.
    pub fn decision_attempt(
        &self,
        request_id: DecisionRequestId,
    ) -> Result<Option<&DecisionAttemptRecord>, CanwuError> {
        self.require_read(&StateKey::core_decisions())?;
        Ok(self.state.current().decisions.attempt(request_id))
    }

    /// Returns one current decision-controller binding after an explicit core read.
    pub fn decision_controller(
        &self,
        id: &str,
    ) -> Result<Option<&DecisionControllerBinding>, CanwuError> {
        self.require_read(&StateKey::core_decisions())?;
        Ok(self.state.current().decisions.controller(id))
    }

    /// Returns one current decision ticket after an explicit core read.
    pub fn decision_ticket(
        &self,
        id: DecisionTicketId,
    ) -> Result<Option<&DecisionTicket>, CanwuError> {
        self.require_read(&StateKey::core_decisions())?;
        Ok(self.state.current().decisions.ticket(id))
    }

    pub fn domain_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.require_domain_record_read(reference)?;
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

    /// Returns the exact evidence reference for the currently visible version
    /// of a domain record.  Strategic aggregation runs after atomic commit,
    /// therefore it cannot use [`Self::proposed_domain_record_version`].
    ///
    /// The current boundary overlay/proposal is preferred, followed by the
    /// runtime's verified current-record provenance index. The index is
    /// maintained at commit time and rebuilt from canonical evidence on
    /// restore, so lookup does not scan retained or archived history.
    pub fn current_domain_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.kind))?;
        let Some(record) = self.domain_record(reference)? else {
            return Ok(None);
        };
        if let Some(proposed) = self.proposal_evidence.and_then(|evidence| {
            evidence.iter().find_map(|item| match item {
                EvidenceRef::DomainRecordVersion(version)
                    if version.record == *reference && version.version == record.version =>
                {
                    Some(version.clone())
                }
                _ => None,
            })
        }) {
            return Ok(Some(proposed));
        }
        let current = super::current_domain_record_version(self.state.runtime(), reference)?;
        if current
            .as_ref()
            .is_some_and(|current| current.version != record.version)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "visible domain-record version disagrees with the runtime provenance index",
            ));
        }
        Ok(current)
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
        self.require_domain_record_read(&reference.record)?;
        if self
            .proposed_domain_record_version(&reference.record)?
            .is_some_and(|proposed| proposed == *reference)
        {
            return Ok(true);
        }
        Ok(!matches!(
            validation::resolve_evidence_reference(
                &validation::RuntimeValidationContext::new(self.state.runtime()),
                &EvidenceRef::DomainRecordVersion(reference.clone()),
            ),
            validation::EvidenceAvailability::Missing
        ))
    }

    /// Checks that an exact domain-record version is both valid evidence and current.
    pub fn domain_record_version_is_current(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<bool, CanwuError> {
        self.require_domain_record_read(&reference.record)?;
        let current = self
            .record_overlay
            .and_then(|overlay| overlay.get(&reference.record))
            .or_else(|| self.state.current().domain_records.get(&reference.record));
        Ok(
            current.is_some_and(|record| record.version == reference.version)
                && self.domain_record_version_evidence_exists(reference)?,
        )
    }

    /// Returns whether a generic evidence identity is retained or archived.
    ///
    /// Domain-record versions proposed earlier in this boundary are visible.
    /// Archived identities count as existing even when their bodies are no
    /// longer retained.
    pub fn evidence_exists(&self, reference: &EvidenceRef) -> Result<bool, CanwuError> {
        match reference {
            EvidenceRef::Command(_) | EvidenceRef::CommandAttempt(_) => {
                self.require_read(&StateKey::core_commands())?;
            }
            EvidenceRef::Event(_) => self.require_read(&StateKey::core_events())?,
            EvidenceRef::Ingress(_) => self.require_read(&StateKey::core_ingress())?,
            EvidenceRef::Boundary(_) | EvidenceRef::RandomDraw(_) => {
                self.require_read(&StateKey::core_evidence())?;
            }
            EvidenceRef::DomainRecordVersion(version) => {
                return self.domain_record_version_evidence_exists(version);
            }
        }
        Ok(!matches!(
            validation::resolve_evidence_reference(
                &validation::RuntimeValidationContext::new(self.state.runtime()),
                reference,
            ),
            validation::EvidenceAvailability::Missing
        ))
    }

    /// Returns when retained or earlier same-boundary evidence first became
    /// authoritative at this proposal-visible cut.
    ///
    /// Archived identity receipts do not retain a precise semantic time, so
    /// they return `None` and callers that require temporal ordering must fail
    /// closed or load the archived evidence body.
    pub fn evidence_time(&self, reference: &EvidenceRef) -> Result<Option<SimTime>, CanwuError> {
        if !self.evidence_exists(reference)? {
            return Ok(None);
        }
        if self
            .proposal_evidence
            .is_some_and(|evidence| evidence.contains(reference))
        {
            return Ok(Some(self.time()));
        }
        Ok(super::retained_evidence_time(
            self.state.runtime(),
            reference,
        ))
    }

    /// Resolves the retained record body for one exact domain-record version.
    ///
    /// Archived receipts prove that a version existed but do not contain its
    /// body, so this returns `None` when the corresponding evidence segment is
    /// not live in the runtime.
    pub fn domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<Option<DomainRecord>, CanwuError> {
        self.require_read(&records::record_state_key(&reference.record.kind))?;
        if let Some(proposed) = self.proposed_domain_record_version(&reference.record)?
            && proposed == *reference
        {
            return Ok(self
                .proposed_records
                .and_then(|records| records.get(&reference.record))
                .or_else(|| {
                    self.record_overlay
                        .and_then(|records| records.get(&reference.record))
                })
                .cloned());
        }
        Ok(retained_domain_record_version(
            self.state.runtime(),
            reference,
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
        self.require_read(&records::record_state_key(kind))?;
        validate_domain_record_page_request(kind, after, limit)?;

        let mut records =
            domain_record_candidates(&self.state.current().domain_records, kind, after, limit);
        for overlay in [self.record_overlay, self.proposed_records]
            .into_iter()
            .flatten()
        {
            for (reference, record) in domain_record_candidates(overlay, kind, after, limit) {
                records.insert(reference, record);
            }
        }
        Ok(records.into_values().take(limit).collect())
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
        self.random_sample_for_operation(
            stream,
            evidence,
            operation_kind,
            application_operation_id,
            target,
            draw_slot,
            upper_exclusive,
            purpose,
        )
        .map(|sample| sample.value)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn random_sample_for_operation(
        &self,
        stream: &RandomStreamKey,
        evidence: EvidenceRef,
        operation_kind: &str,
        application_operation_id: &str,
        target: RandomOperationTarget,
        draw_slot: u32,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<super::RandomSample, CanwuError> {
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
        session.borrow_mut().sample_for_operation(
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

    fn require_domain_record_read(&self, reference: &DomainRecordRef) -> Result<(), CanwuError> {
        let exact = records::record_state_key(&reference.kind);
        if self.allowed_reads.is_some_and(|reads| {
            !reads.contains(&exact) && !reads.contains(&StateKey::core_domain_records())
        }) {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateRead,
                format!(
                    "{} did not declare read access to {}.{}",
                    self.reader.unwrap_or("internal system"),
                    exact.namespace,
                    exact.name
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

impl super::PluginArchiveObjectProvider for SimulationView<'_> {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.plugin_archive_object(namespace, object_id)
    }
}
