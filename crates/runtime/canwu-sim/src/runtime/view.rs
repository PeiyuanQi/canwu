use super::{
    ActorKnowledge, Army, ArmyId, BoundaryId, BoundaryKnowledgeChange, CanwuError, CauseRef,
    CommandId, CommandRecord, CreatedPerson, DecisionAttemptRecord, DecisionControllerBinding,
    DecisionRequestId, DecisionTicket, DecisionTicketId, DomainRecord, DomainRecordKind,
    DomainRecordRef, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    EntityRef, ErrorCode, EventId, EvidenceRef, Government, GovernmentId, HashSet, IngressId,
    IngressPayload, IngressQueueKey, IngressRecord, KnowledgeHolderRef, KnowledgeQuery,
    KnowledgeRecord, KnowledgeRecordId, Person, PersonAvailability, PersonId, PluginComponentKey,
    PluginComponentRecord, RandomOperationTarget, RandomStreamKey, RefCell, ReservationAllocation,
    ReservationRef, Route, RouteId, RuntimeCurrentState, RuntimeEvidence, RuntimeState, SimEvent,
    SimTime, StateKey, Territory, TerritoryId, TypedDomainRecordRef, Value, component_key,
    domain_record_candidates, live_current_domain_record_version, random, records,
    retained_domain_record_version, retained_evidence_time, validate_domain_record_page_request,
    validation,
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
    /// Registered domain record schemas, for the knowledge-holder rule.
    pub(super) record_schemas: &'a records::DomainRecordSchemas,
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
    pub(super) transitions: Option<&'a super::transitions::BoundaryTransitionLedger>,
    /// Summary of the boundary being settled, for boundary-system views.
    pub(super) current_boundary: Option<&'a super::BoundarySummary>,
    /// How far back this reader may look: the boundary system's own declared
    /// horizon, or zero for views that cannot declare the read.
    pub(super) boundary_schedule_horizon: canwu_time::SimDuration,
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

    /// Returns a person's record in the `WorldSnapshot` projection.
    ///
    /// `None` means the projection holds no record for the ID. A person that
    /// exists only in the entity registry, such as one created from an
    /// unplaced [`crate::PersonDraft`], has none; bind its identity with
    /// [`Self::persons_created_by_correlation`] and read its availability
    /// with [`Self::person_availability`].
    pub fn person(&self, id: PersonId) -> Result<Option<&Person>, CanwuError> {
        self.require_read(&StateKey::core_people())?;
        Ok(self.state.current().people.get(&id))
    }

    /// Whether a boundary's knowledge publication to `holder` would be
    /// accepted at this view's state: the holder rule the engine validates a
    /// `PublishKnowledge` directive with (an existing person who is not dead,
    /// an existing army or government, or an active, undeleted domain entity
    /// whose schema allows knowledge holders; no other core entity kind).
    ///
    /// A system that publishes to holders its records name checks this first
    /// and leaves an ineligible holder unpublished instead of failing its
    /// boundary.
    ///
    /// It reads person and entity state (the entity registry, people, armies,
    /// governments, person availability, and domain records with their
    /// schemas) without a read declaration. That is safe because the answer is
    /// exactly the holder check the engine applies, against the same state, to
    /// any `PublishKnowledge` directive the system proposes: a system that may
    /// publish already depends on that state through the validation of its
    /// own output, and the read reveals one yes or no for a holder the caller
    /// names, nothing it could not learn by proposing the publication.
    pub fn knowledge_holder_accepts_publication(
        &self,
        holder: &KnowledgeHolderRef,
    ) -> Result<bool, CanwuError> {
        Ok(super::knowledge::validate_holder_for_publication(
            holder,
            self.state.current(),
            self.record_schemas,
        )
        .is_ok())
    }

    /// Returns committed core availability for a person after an explicit
    /// `canwu.core.person_availability` read. `None` means alive and free.
    pub fn person_availability(
        &self,
        id: PersonId,
    ) -> Result<Option<&PersonAvailability>, CanwuError> {
        self.require_read(&StateKey::core_person_availability())?;
        Ok(self.state.current().person_availability.get(&id))
    }

    /// Finds committed person creations produced by one plugin with an exact
    /// correlation, so the proposing system can bind engine-allocated IDs at
    /// a later boundary. The lookup reads the persisted created-person
    /// registry, so it does not depend on retained evidence.
    pub fn persons_created_by_correlation(
        &self,
        plugin: &str,
        correlation: &str,
    ) -> Result<Vec<CreatedPerson>, CanwuError> {
        self.require_read(&StateKey::core_people())?;
        Ok(self
            .state
            .current()
            .created_persons
            .iter()
            .filter(|created| created.plugin == plugin && created.correlation == correlation)
            .cloned()
            .collect())
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
    ///
    /// The committed ledgers keep a count per namespace, so the read costs
    /// the records the settling boundary has staged, not the records held.
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

    /// Returns the global IDs of a holder's current records of one schema
    /// kind, after an explicit `canwu.core.knowledge` read, so a publishing
    /// system can name the records its next record supersedes.
    ///
    /// A record is current when no record of the holder supersedes it. Only
    /// records of `kind` are returned, and an empty `subjects` selects every
    /// one of them. A record matches when it carries every listed subject,
    /// not when its subjects equal the list. A publisher that keeps several
    /// lines of records for one holder therefore gives each line a subject
    /// the others lack: a line whose subjects are a subset of another line's
    /// cannot be selected apart from it. Subjects are committed state;
    /// producer and correlation are recorded only on boundary evidence and
    /// are not keys. A schema kind has one owner plugin, so lines need
    /// telling apart only between systems or schema versions of one plugin.
    ///
    /// The read sees what the relation rule of
    /// [`crate::BoundaryDirective::PublishKnowledge`] admits: records committed
    /// by an earlier boundary, whatever their learned time, and records the
    /// settling boundary published with `SameBoundary` visibility in an
    /// earlier phase. Every returned ID is therefore a valid `supersedes` or
    /// `contradicts` target for a record this system publishes to the same
    /// holder with the same kind, and the IDs are already in the ascending
    /// order a draft requires.
    ///
    /// The answer comes from the committed knowledge ledger and the boundary
    /// being settled, never from retained evidence, so a live run, a run that
    /// sealed its evidence, a restored run, and exact replay return the same
    /// IDs.
    ///
    /// At most [`crate::KnowledgeLimitsV1::relations_per_record`] IDs are
    /// returned, the number one record may supersede. When more records
    /// match, the newest are returned, so a publisher always sees its own
    /// latest record, and [`crate::KnowledgeHeads::truncated`] is set. That is
    /// not an error: the publisher supersedes what was returned, and the
    /// older current records drain over its later publications.
    ///
    /// The read never walks the holder's ledger. Each holder ledger keeps an
    /// index of its current records by kind and by subject (see
    /// [`crate::HolderLedger::current_records`]); the read walks, newest
    /// first, the current records of `kind` that carry the listed subject the
    /// fewest of them carry, and stops after one match more than the bound.
    /// With no subjects or one subject it visits only matches; superseded
    /// records are never visited, however many the holder keeps. The records
    /// the settling boundary published to the holder are read as well.
    ///
    /// The IDs are global ledger IDs for boundary systems. Actor-facing reads
    /// keep using holder-local IDs.
    pub fn knowledge_heads(
        &self,
        holder: &KnowledgeHolderRef,
        kind: &canwu_core::KnowledgeRecordKind,
        subjects: &[canwu_knowledge::KnowledgeSubject],
    ) -> Result<super::KnowledgeHeads, CanwuError> {
        self.require_read(&StateKey::core_knowledge())?;
        let limit = super::KnowledgeLimitsV1::CURRENT.relations_per_record;
        if subjects.len() > limit {
            return Err(CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "knowledge head subject filter exceeds the record relation limit",
            ));
        }
        Ok(super::knowledge::current_heads_at_cut(
            holder,
            kind,
            subjects,
            &self.state.current().knowledge.records,
            self.knowledge_overlay,
            limit,
        ))
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

    /// Whether a boundary system may schedule ingress
    /// (`BoundaryDirective::ScheduleIngress` or `SchedulePluginIngress`) in
    /// this run. A run that holds direct command history cannot add
    /// canonical ingress, and a boundary whose system schedules some then
    /// fails with `MixedCommandIngress`; a system that can do without the
    /// packet checks this first. The answer depends only on the run's
    /// command history, so a restore, a seal, and an exact replay give the
    /// same one.
    #[must_use]
    pub fn can_schedule_ingress(&self) -> bool {
        !validation::runtime_has_unqueued_command_history(self.state.runtime())
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

    /// A retained record that is still queued, or that a terminal
    /// cancellation withdrew, was never admitted and is not durable evidence
    /// of a delivered packet.
    fn ingress_is_pending_or_cancelled(&self, record: &IngressRecord) -> bool {
        let scheduler = &self.state.runtime().scheduler;
        scheduler.cancelled_ingress.contains(&record.id)
            || scheduler
                .pending_ingress
                .contains(&IngressQueueKey::from_record(record))
    }

    /// Lists the still-pending plugin ingress that this boundary system's own
    /// plugin scheduled inside the engine and may withdraw with
    /// [`crate::BoundaryDirective::CancelPluginIngress`], in ingress-ID order.
    ///
    /// Only items due strictly after the view time are listed. Items that a
    /// boundary directive scheduled earlier in the current boundary appear
    /// from the next boundary; items generated by this plugin's commands are
    /// listed as soon as they are queued. Views that are not bound to a
    /// boundary system list nothing.
    pub fn cancellable_plugin_ingress(&self) -> Result<Vec<&IngressRecord>, CanwuError> {
        self.require_read(&StateKey::core_ingress())?;
        let Some(owner) = self.ingress_plugin else {
            return Ok(Vec::new());
        };
        let now = self.state.now();
        let evidence = self.state.evidence();
        let mut records = Vec::new();
        for key in &self.state.runtime().scheduler.pending_ingress {
            if key.due_at <= now {
                continue;
            }
            let Some(record) = evidence.retained_ingress(key.id) else {
                continue;
            };
            let IngressPayload::Plugin { plugin, .. } = &record.payload else {
                continue;
            };
            let issued_by_owner = match &record.cause {
                Some(CauseRef::Command(_)) => plugin == owner,
                Some(CauseRef::Boundary(boundary)) => evidence
                    .retained_boundary(*boundary)
                    .is_some_and(|boundary| {
                        boundary.generated_ingress.iter().any(|generation| {
                            generation.ingress == record.id && generation.plugin == owner
                        })
                    }),
                Some(CauseRef::System(_) | CauseRef::Event(_)) | None => false,
            };
            if issued_by_owner {
                records.push(record);
            }
        }
        records.sort_by_key(|record| record.id);
        Ok(records)
    }

    /// Returns the boundaries of the half-open window `from..to` after an
    /// explicit `canwu.core.boundary_schedule` read: those settled in it, the
    /// boundary being settled when this view belongs to a boundary system and
    /// its time is in the window, and those still queued in it, so a system
    /// can count a period's boundaries before it carries work into one more.
    ///
    /// The settled list is replay-stable: it is the same in a live run, a
    /// sealed run, a restored run, and its exact replay. A system may look
    /// back only as far as its own
    /// [`super::BoundarySystemContract::boundary_schedule_horizon`]; the run
    /// keeps settled boundaries for the longest horizon any system declares.
    /// Queued boundaries are the distinct due times in the window in the
    /// canonical queues at or after the view time, so they are a lower bound
    /// (see [`super::BoundaryWindow`]). Work proposed by the current boundary
    /// is queued when the boundary commits and is listed from the next
    /// boundary on; ingress queued by a command handler is listed as soon as
    /// it is queued.
    ///
    /// # Errors
    ///
    /// Returns `ValueOutOfRange` when `to` precedes `from`, and
    /// `EvidenceUnavailable` when `from` lies further back from the view time
    /// than this system's declared horizon.
    ///
    /// Only phased boundary systems may declare the read. It names other
    /// plugins' queued packet types, classes, and origins, never payloads, so
    /// a system can count sources it does not own; `canwu.core.ingress`
    /// resolves only the reader's own admitted packets.
    pub fn boundaries_between(
        &self,
        from: SimTime,
        to: SimTime,
    ) -> Result<super::BoundaryWindow, CanwuError> {
        self.require_read(&StateKey::core_boundary_schedule())?;
        super::boundary_window::boundary_window(
            self.state.runtime(),
            self.boundary_schedule_horizon,
            from,
            to,
            self.state.now(),
            self.current_boundary,
        )
    }

    /// Lists the pending transition manifests that this boundary system's
    /// plugin coordinates or participates in, in manifest-ID order, after an
    /// explicit `canwu.core.transitions` read.
    ///
    /// A manifest's expected versions name other plugins' records, so the
    /// list is relative to the reading plugin; other plugins' manifests and
    /// views not bound to a boundary system list nothing. Inside a boundary,
    /// a manifest registered by an earlier phase is listed from the next phase
    /// on, and a manifest that settled at phase 11 is no longer listed. A
    /// participant stages for the manifests whose `ready_at` is the current
    /// boundary and that list its plugin.
    pub fn transition_manifests(
        &self,
    ) -> Result<Vec<&super::PendingTransitionManifest>, CanwuError> {
        self.require_read(&StateKey::core_transitions())?;
        let (Some(reader), Some(ledger)) = (self.ingress_plugin, self.transitions) else {
            return Ok(Vec::new());
        };
        Ok(ledger
            .pending()
            .filter(|manifest| manifest.involves(reader))
            .collect())
    }

    /// Returns the audits of the transition manifests that settled at phase 11
    /// of the current boundary and that this boundary system's plugin
    /// coordinates or participates in, in manifest-ID order, after an
    /// explicit `canwu.core.transitions` read.
    ///
    /// A failed audit fails the boundary instead of leaving a record, so every
    /// outcome is [`crate::TransitionAuditOutcome::Committed`] or
    /// [`crate::TransitionAuditOutcome::Expired`]. Earlier phases and views
    /// not bound to a boundary system see no audits; hosts read settled
    /// audits from [`crate::BoundaryRecord::transition_audits`] and
    /// [`crate::BoundaryReceipt::transition_audits`].
    pub fn transition_audits(&self) -> Result<Vec<&super::TransitionAuditRecord>, CanwuError> {
        self.require_read(&StateKey::core_transitions())?;
        let (Some(reader), Some(ledger)) = (self.ingress_plugin, self.transitions) else {
            return Ok(Vec::new());
        };
        Ok(ledger
            .audits()
            .iter()
            .filter(|audit| audit.involves(reader))
            .collect())
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
        if self.ingress_is_pending_or_cancelled(record) {
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
        if self.ingress_is_pending_or_cancelled(record) {
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

    /// Returns `canonical_hash(domain, &(due_at, payload))` of an admitted,
    /// boundary-generated plugin packet, without exposing the payload, when
    /// [`Self::plugin_ingress_matches`] matches it to `plugin` and
    /// `packet_type`; `None` when it does not or its record is not retained.
    ///
    /// A reader that will later need to recognize the packet's exact content
    /// keeps the hash in its own state and compares it with
    /// `canonical_hash(domain, &(expected_due_at, &expected_payload))`, so the
    /// later check reads no retained evidence. A reader whose decision must not
    /// depend on sealing takes the hash only of packets its boundary admits,
    /// which every run holds.
    pub fn plugin_ingress_payload_hash(
        &self,
        id: IngressId,
        plugin: &str,
        packet_type: &str,
        domain: &str,
    ) -> Result<Option<String>, CanwuError> {
        if !self.plugin_ingress_matches(id, plugin, packet_type)? {
            return Ok(None);
        }
        let Some(record) = self.state.evidence().retained_ingress(id) else {
            return Ok(None);
        };
        let IngressPayload::Plugin { payload, .. } = &record.payload else {
            return Ok(None);
        };
        super::canonical_hash(domain, &(record.due_at, payload)).map(Some)
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
        Ok(self.proposed_version_evidence(reference))
    }

    /// Same-boundary proposal evidence for one record, without a read check;
    /// callers gate access before using it.
    fn proposed_version_evidence(
        &self,
        reference: &DomainRecordRef,
    ) -> Option<DomainRecordVersionRef> {
        self.proposal_evidence.and_then(|evidence| {
            evidence.iter().find_map(|item| match item {
                EvidenceRef::DomainRecordVersion(version) if version.record == *reference => {
                    Some(version.clone())
                }
                _ => None,
            })
        })
    }

    /// Returns the exact evidence reference for the currently visible version
    /// of a domain record.  Strategic aggregation runs after atomic commit,
    /// therefore it cannot use [`Self::proposed_domain_record_version`].
    ///
    /// The current boundary overlay/proposal is preferred, followed by the
    /// runtime's verified current-record provenance index. The index is
    /// maintained at commit time and rebuilt from canonical evidence on
    /// restore, so lookup does not scan retained or archived history, and its
    /// answer does not depend on how much evidence a seal kept. Either the
    /// exact record-kind read or the administrative domain-record read grants
    /// access.
    pub fn current_domain_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
        self.require_domain_record_read(reference)?;
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
    /// or archived runtime evidence. An archived version resolves through its
    /// own kept receipt. Each seal keeps the receipt of every record's current
    /// version, including retired and deleted records, and of any other version
    /// only if a live dependency declares it at that seal. Exact replay always
    /// finds that other version, so a rule whose outcome must match exact
    /// replay should use [`Self::replay_stable_domain_record_version`]
    /// instead. Either the exact record-kind read or the administrative
    /// domain-record read grants access.
    pub fn domain_record_version_evidence_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<bool, CanwuError> {
        // The proposal lookup must not re-require the exact kind, or the
        // administrative read could never resolve evidence.
        self.require_domain_record_read(&reference.record)?;
        if self
            .proposed_version_evidence(&reference.record)
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
    /// they return `None` where exact replay, which retains every item, returns
    /// the time. A record's committed current version is the exception: it
    /// keeps its establishment time after its establishing boundary is sealed.
    /// A rule whose outcome must match exact replay therefore reads an exact
    /// version's time through [`Self::replay_stable_domain_record_version`].
    /// Generic evidence has no such read: after a seal, both its existence
    /// and its time depend on what the seal kept.
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
    /// not live in the runtime. The committed current version resolves to its
    /// live body even after its establishing boundary is sealed.
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

    /// Resolves an exact domain-record version whose body and establishment
    /// time do not depend on how much evidence a seal kept.
    ///
    /// Returns them for the committed current version of a record, for a
    /// version proposed earlier in this boundary and not superseded since, and
    /// for an initial-scenario version. Any other version returns `None` even while it is retained,
    /// because `seal_evidence` can remove its existence proof, body, and time
    /// while exact replay keeps them. A rule whose outcome must match exact
    /// replay relies on this instead of [`Self::domain_record_version`] and
    /// [`Self::evidence_time`]. The committed current version still resolves
    /// after an earlier system in this boundary proposes a newer one. Either
    /// the exact record-kind read or the administrative domain-record read
    /// grants access.
    pub fn replay_stable_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<Option<(DomainRecord, SimTime)>, CanwuError> {
        self.require_domain_record_read(&reference.record)?;
        // A proposal still visible in an overlay resolves from it. A change
        // this boundary has already committed is the current version below.
        if self.proposal_evidence.is_some_and(|evidence| {
            evidence.contains(&EvidenceRef::DomainRecordVersion(reference.clone()))
        }) && let Some(record) = [self.proposed_records, self.record_overlay]
            .into_iter()
            .flatten()
            .find_map(|records| {
                records
                    .get(&reference.record)
                    .filter(|record| record.version == reference.version)
            })
        {
            return Ok(Some((record.clone(), self.time())));
        }
        let runtime = self.state.runtime();
        if !matches!(
            reference.established_by,
            DomainRecordVersionSource::InitialScenario
        ) && live_current_domain_record_version(runtime, reference).is_none()
        {
            return Ok(None);
        }
        Ok(
            retained_domain_record_version(runtime, reference).zip(retained_evidence_time(
                runtime,
                &EvidenceRef::DomainRecordVersion(reference.clone()),
            )),
        )
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
    ///
    /// The read scans retained boundary evidence. After `seal_evidence`
    /// removes the publishing boundary it returns nothing for that boundary,
    /// while exact replay, which retains every boundary, still returns the
    /// change. It must not drive a decision that has to replay exactly once
    /// the publishing boundary may have been sealed. To find the records a
    /// new record supersedes, use [`Self::knowledge_heads`], which reads
    /// committed state.
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
    ///
    /// Like [`Self::knowledge_changes_by_correlation`], the read scans
    /// retained boundary evidence and returns nothing for a sealed boundary,
    /// so it must not drive a decision that has to replay exactly once the
    /// publishing boundary may have been sealed. To find the records a new
    /// record supersedes, use [`Self::knowledge_heads`].
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

    /// Makes an operation-keyed random draw and returns its sample. Use
    /// [`Self::random_draw_for_operation`] when the caller also needs the
    /// ID of the draw's evidence.
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
        self.random_draw_for_operation(
            stream,
            evidence,
            operation_kind,
            application_operation_id,
            target,
            draw_slot,
            upper_exclusive,
            purpose,
        )
        .map(|draw| draw.sample)
    }

    /// Makes an operation-keyed random draw and returns its sample with the
    /// ID of its draw evidence, so a record the system writes can cite
    /// the draw it was computed from.
    ///
    /// The draw is addressed by the system's plugin, `operation_kind`,
    /// `application_operation_id`, `target`, and `draw_slot` on `stream`.
    ///
    /// An address the run has already drawn returns the recorded value and
    /// the ID of that draw without drawing again, when `evidence`,
    /// `upper_exclusive`, and `purpose` are the ones it was drawn with. Only
    /// the evidence reference has to be the same: the answer does not depend
    /// on whether the draw's record or its evidence is still retained, so a
    /// run whose evidence was sealed answers as its unsealed replay does.
    ///
    /// An address that has not been drawn is drawn now, and returns the ID
    /// its record has once the boundary being settled commits; see
    /// [`super::RandomOperationDraw::draw_id`]. A fresh draw needs operation
    /// evidence that exists at the reader's cut: retained evidence, or
    /// evidence produced earlier in this boundary. Evidence that a seal has
    /// archived is refused, although an unsealed run with the same history
    /// would accept it. A system that may draw an address for the first time
    /// after a seal, and must then behave as its unsealed replay, cites
    /// evidence that survives a seal, such as the initial-scenario version
    /// of a record, or evidence newer than the seal.
    ///
    /// A handler whose fresh draw is refused for sealed evidence must return
    /// the error, which fails the boundary. A handler that swallows
    /// `InvalidRandomOperationEvidence` and carries on settles a boundary
    /// that the run's own exact replay rejects, because the replay retains
    /// the evidence and makes the draw. A run restored from its sealed
    /// segments retains that evidence again too, so it accepts the draw the
    /// live sealed run refused.
    ///
    /// # Errors
    ///
    /// In a view with a random session, which a boundary system's view has,
    /// the errors are checked in this order: `UndeclaredRandomStream` when
    /// the system did not declare `stream`; `InvalidRandomDraw` for a
    /// malformed address, bound, or purpose; `RandomOperationConflict` when
    /// the address was already drawn with different evidence, bound, or
    /// purpose; `InvalidRandomOperationEvidence` when the address has not
    /// been drawn and `evidence` is neither retained nor produced earlier in
    /// this boundary; and `IdentifierExhausted` when the run has no draw ID
    /// left for a fresh draw.
    ///
    /// A view without a random session, such as a command handler's, never
    /// draws. There the evidence is still checked first: the call returns
    /// `InvalidRandomOperationEvidence` when `evidence` is not retained, and
    /// `UndeclaredRandomStream` otherwise.
    #[allow(clippy::too_many_arguments)]
    pub fn random_draw_for_operation(
        &self,
        stream: &RandomStreamKey,
        evidence: EvidenceRef,
        operation_kind: &str,
        application_operation_id: &str,
        target: RandomOperationTarget,
        draw_slot: u32,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<super::RandomOperationDraw, CanwuError> {
        // Evidence a new draw may cite: retained, or produced earlier in this
        // boundary.
        let available = |evidence: &EvidenceRef| {
            self.proposal_evidence
                .is_some_and(|values| values.contains(evidence))
                || validation::resolve_evidence_reference(
                    &validation::RuntimeValidationContext::new(self.state.runtime()),
                    evidence,
                ) == validation::EvidenceAvailability::Retained
        };
        let Some(session) = &self.random_session else {
            if !available(&evidence) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidRandomOperationEvidence,
                    "operation-keyed random draw references unavailable evidence",
                ));
            }
            return Err(CanwuError::new(
                ErrorCode::UndeclaredRandomStream,
                format!(
                    "system {} has no declared random streams",
                    self.reader.unwrap_or("unscoped caller")
                ),
            ));
        };
        session.borrow_mut().draw_for_operation(
            stream,
            evidence,
            operation_kind,
            application_operation_id,
            target,
            draw_slot,
            upper_exclusive,
            purpose,
            &available,
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
