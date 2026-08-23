use super::event_payloads::{
    ArmyArrived, KnowledgeUpdated, LetterDelivered, PersonArrived, ReportDispatched,
    RuntimeEventPayload,
};
use super::{
    ActorKnowledge, ArmyId, ArmyKnowledge, AssertUnwindSafe, BTreeMap, BoundaryId, CanwuError,
    CauseRef, ClockTransactionCheckpoint, CommitmentDomains, DeterministicRng, EntityRef,
    ErrorCode, EstimateRange, EventId, EventKind, KnowledgeSource, LetterId, LetterStatus,
    PendingBoundaryRandomDraw, PersonId, RandomDrawAddress, RandomDrawId, RandomDrawOutcome,
    RandomDrawProducer, RandomDrawRecord, RandomStreamKey, RuntimeValidationContext,
    ScheduledBatchTransactionCheckpoint, SimDuration, SimEvent, SimTime, Simulation,
    SimulationView, SimulationViewState, StateKey, SystemDirective, TerritoryId, catch_unwind,
    claim_counter, random, validate_directives_with_context,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(super) struct ScheduleKey {
    pub(super) at: SimTime,
    pub(super) sequence: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ScheduledAction {
    ArmyArrival {
        army: ArmyId,
        destination: TerritoryId,
        order_event: EventId,
        correlation_id: u64,
    },
    PersonArrival {
        person: PersonId,
        destination: TerritoryId,
        order_event: EventId,
        cargo: Vec<LetterId>,
        correlation_id: u64,
    },
    KnowledgeReport {
        recipient: PersonId,
        army: ArmyId,
        location: TerritoryId,
        observed_at: SimTime,
        dispatch_event: EventId,
        correlation_id: u64,
    },
    PluginDirective {
        plugin: String,
        directive: Box<SystemDirective>,
        allowed_writes: Vec<StateKey>,
        cause: CauseRef,
        correlation_id: u64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct ScheduledRecord {
    pub(super) key: ScheduleKey,
    pub(super) action: ScheduledAction,
}

impl Simulation {
    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.ensure_runtime_ready()?;
        if duration.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "simulation time cannot advance by a negative duration",
            ));
        }
        let target = self
            .state
            .scheduler
            .now
            .checked_add(duration)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDuration,
                    "simulation target time exceeds the supported range",
                )
            })?;
        self.ensure_legacy_advance_does_not_cross_ingress(target)?;
        self.advance_to(target)
    }

    pub fn step(&mut self) -> Result<Vec<SimEvent>, CanwuError> {
        self.ensure_runtime_ready()?;
        if self.state.scheduler.pending_ingress.first().is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "pending canonical ingress requires step_canonical",
            ));
        }
        let Some(next_time) = self.state.scheduler.actions.keys().next().map(|key| key.at) else {
            return Ok(Vec::new());
        };
        self.advance_to(next_time)
    }

    pub fn advance_until<F>(
        &mut self,
        maximum: SimDuration,
        mut condition: F,
    ) -> Result<Vec<SimEvent>, CanwuError>
    where
        F: FnMut(&Self) -> bool,
    {
        self.ensure_runtime_ready()?;
        if maximum.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "advance_until maximum cannot be negative",
            ));
        }
        let target = self
            .state
            .scheduler
            .now
            .checked_add(maximum)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDuration,
                    "advance_until target time exceeds the supported range",
                )
            })?;
        self.ensure_legacy_advance_does_not_cross_ingress(target)?;
        let start = self.state.evidence.events.len();
        while self.state.scheduler.now < target && !condition(self) {
            let next_time = self
                .state
                .scheduler
                .actions
                .keys()
                .next()
                .map_or(target, |key| key.at.min(target));
            self.advance_to(next_time)?;
            if next_time == target {
                break;
            }
        }
        Ok(self.state.evidence.events[start..].to_vec())
    }

    pub(super) fn ensure_legacy_advance_does_not_cross_ingress(
        &self,
        target: SimTime,
    ) -> Result<(), CanwuError> {
        if self
            .state
            .scheduler
            .pending_ingress
            .first()
            .is_some_and(|key| key.due_at <= target)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "legacy time advancement cannot cross pending canonical ingress; use advance_canonical",
            ));
        }
        Ok(())
    }

    pub(super) fn advance_to(&mut self, target: SimTime) -> Result<Vec<SimEvent>, CanwuError> {
        let start = self.state.evidence.events.len();
        while let Some(boundary_time) = self.state.scheduler.actions.keys().next().map(|key| key.at)
            && boundary_time <= target
        {
            let transaction = ScheduledBatchTransactionCheckpoint::capture(&self.state);
            let result = (|| {
                self.invalidate_commitments(CommitmentDomains::SCHEDULER);
                self.state.scheduler.now = boundary_time;
                while self
                    .state
                    .scheduler
                    .actions
                    .first_key_value()
                    .is_some_and(|(key, _)| key.at == boundary_time)
                {
                    let (_, action) = self
                        .state
                        .scheduler
                        .actions
                        .pop_first()
                        .expect("scheduler was checked as non-empty");
                    self.execute_scheduled(action)?;
                }
                self.state.metadata.plugin_registration_closed = true;
                self.refresh_checkpoint_hash()
            })();
            if let Err(error) = result {
                transaction.restore(&mut self.state);
                return Err(error);
            }
        }
        let transaction = ClockTransactionCheckpoint::capture(&self.state);
        self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        self.state.scheduler.now = target;
        self.state.metadata.plugin_registration_closed = true;
        if let Err(error) = self.refresh_checkpoint_hash() {
            transaction.restore(&mut self.state);
            return Err(error);
        }
        Ok(self.state.evidence.events[start..].to_vec())
    }

    pub(super) fn advance_to_before_boundary(&mut self, target: SimTime) -> Result<(), CanwuError> {
        while let Some(next) = self.state.scheduler.actions.keys().next().map(|key| key.at)
            && next < target
        {
            self.advance_to(next)?;
        }
        self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        self.state.scheduler.now = target;
        self.state.metadata.plugin_registration_closed = true;
        self.refresh_checkpoint_hash()
    }

    pub(super) fn execute_scheduled_at(&mut self, at: SimTime) -> Result<(), CanwuError> {
        if self
            .state
            .scheduler
            .actions
            .first_key_value()
            .is_some_and(|(key, _)| key.at == at)
        {
            self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        }
        while self
            .state
            .scheduler
            .actions
            .first_key_value()
            .is_some_and(|(key, _)| key.at == at)
        {
            let (_, action) = self
                .state
                .scheduler
                .actions
                .pop_first()
                .expect("scheduler was checked as non-empty");
            self.execute_scheduled(action)?;
        }
        Ok(())
    }

    fn execute_scheduled(&mut self, action: ScheduledAction) -> Result<(), CanwuError> {
        match action {
            ScheduledAction::ArmyArrival {
                army,
                destination,
                order_event,
                correlation_id,
            } => self.execute_arrival(army, destination, order_event, correlation_id),
            ScheduledAction::PersonArrival {
                person,
                destination,
                order_event,
                cargo,
                correlation_id,
            } => self.execute_person_arrival(
                person,
                destination,
                order_event,
                &cargo,
                correlation_id,
            ),
            ScheduledAction::KnowledgeReport {
                recipient,
                army,
                location,
                observed_at,
                dispatch_event,
                correlation_id,
            } => {
                self.update_army_knowledge(
                    recipient,
                    army,
                    location,
                    observed_at,
                    KnowledgeSource::Report {
                        source_event: dispatch_event,
                    },
                    850,
                );
                self.emit(
                    KnowledgeUpdated {
                        recipient,
                        army,
                        known_location: location,
                    }
                    .into_kind(),
                    vec![EntityRef::Person(recipient), EntityRef::Army(army)],
                    format!(
                        "Person {recipient} received a report locating army {army} at {location}"
                    ),
                    Some(CauseRef::Event(dispatch_event)),
                    correlation_id,
                )?;
                Ok(())
            }
            ScheduledAction::PluginDirective {
                plugin,
                directive,
                allowed_writes,
                cause,
                correlation_id,
            } => {
                let directives = vec![*directive];
                validate_directives_with_context(
                    &RuntimeValidationContext::new(&self.state),
                    &plugin,
                    &allowed_writes,
                    &self.plugins.state_owners,
                    &self.plugins.record_schemas,
                    &directives,
                )?;
                self.apply_directives(&plugin, directives, &allowed_writes, &cause, correlation_id)
            }
        }
    }

    fn execute_arrival(
        &mut self,
        army: ArmyId,
        destination: TerritoryId,
        order_event: EventId,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        self.invalidate_commitments(CommitmentDomains::WORLD);
        let commander = {
            let army_state = self.state.current.armies.get_mut(&army).ok_or_else(|| {
                CanwuError::new(ErrorCode::ArmyNotFound, "scheduled army no longer exists")
            })?;
            army_state.location = destination;
            army_state.transit = None;
            army_state.commander
        };
        let arrival_event = self.emit(
            ArmyArrived {
                army,
                territory: destination,
            }
            .into_kind(),
            vec![EntityRef::Army(army), EntityRef::Territory(destination)],
            format!("Army {army} arrived in territory {destination}"),
            Some(CauseRef::Event(order_event)),
            correlation_id,
        )?;

        self.update_army_knowledge(
            commander,
            army,
            destination,
            self.state.scheduler.now,
            KnowledgeSource::CommandResponsibility,
            1000,
        );
        self.emit(
            KnowledgeUpdated {
                recipient: commander,
                army,
                known_location: destination,
            }
            .into_kind(),
            vec![EntityRef::Person(commander), EntityRef::Army(army)],
            format!("Commander {commander} learned that army {army} arrived at {destination}"),
            Some(CauseRef::Event(arrival_event)),
            correlation_id,
        )?;

        let recipients: Vec<_> = self
            .state
            .current
            .people
            .keys()
            .copied()
            .filter(|person| *person != commander)
            .collect();
        for recipient in recipients {
            let (draw_id, jitter) = self.draw_random(
                &random::core_report_delay_stream(),
                12 * 60,
                "knowledge report delivery jitter",
                RandomDrawProducer::CoreSystem {
                    system: "canwu.core.knowledge-report-delay".to_owned(),
                },
                CauseRef::Event(arrival_event),
                correlation_id,
            )?;
            let jitter_minutes =
                i64::try_from(jitter).expect("report jitter is bounded to a small integer");
            let arrives_at = self
                .state
                .scheduler
                .now
                .checked_add(SimDuration::hours(36))
                .and_then(|time| time.checked_add(SimDuration::minutes(jitter_minutes)))
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "knowledge report arrival time exceeds the supported range",
                    )
                })?;
            let dispatch_event = self.emit(
                ReportDispatched {
                    recipient,
                    army,
                    arrives_at,
                }
                .into_kind(),
                vec![EntityRef::Person(recipient), EntityRef::Army(army)],
                format!("A report about army {army} was dispatched to person {recipient}"),
                Some(CauseRef::Event(arrival_event)),
                correlation_id,
            )?;
            self.record_random_outcome(
                draw_id,
                RandomDrawOutcome::KnowledgeReportDelivery {
                    recipient,
                    army,
                    dispatch_event,
                    arrives_at,
                },
            )?;
            self.schedule_at(
                arrives_at,
                ScheduledAction::KnowledgeReport {
                    recipient,
                    army,
                    location: destination,
                    observed_at: self.state.scheduler.now,
                    dispatch_event,
                    correlation_id,
                },
            )?;
        }
        Ok(())
    }

    fn execute_person_arrival(
        &mut self,
        person: PersonId,
        destination: TerritoryId,
        order_event: EventId,
        cargo: &[super::LetterId],
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        self.invalidate_commitments(CommitmentDomains::WORLD);
        {
            let person_state = self.state.current.people.get_mut(&person).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::EntityNotFound,
                    "scheduled person no longer exists",
                )
            })?;
            let transit = person_state.transit.take().ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "person arrival has no matching transit",
                )
            })?;
            if transit.to != destination || transit.arrives_at != self.state.scheduler.now {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "person arrival disagrees with its transit state",
                ));
            }
            person_state.current_location = destination;
        }
        let arrival_event = self.emit(
            PersonArrived {
                person,
                territory: destination,
            }
            .into_kind(),
            vec![EntityRef::Person(person), EntityRef::Territory(destination)],
            format!("Person {person} arrived in territory {destination}"),
            Some(CauseRef::Event(order_event)),
            correlation_id,
        )?;

        for letter_id in cargo {
            self.settle_letter_at_arrival(
                *letter_id,
                person,
                destination,
                arrival_event,
                correlation_id,
            )?;
        }
        let waiting_letters: Vec<_> = self
            .state
            .current
            .letters
            .values()
            .filter(|letter| {
                letter.status == LetterStatus::HeldAtLocation
                    && letter.location == Some(destination)
                    && letter.recipient == person
            })
            .map(|letter| letter.id)
            .collect();
        for letter_id in waiting_letters {
            self.deliver_letter(
                letter_id,
                person,
                destination,
                arrival_event,
                correlation_id,
            )?;
        }
        Ok(())
    }

    fn settle_letter_at_arrival(
        &mut self,
        letter_id: super::LetterId,
        carrier: PersonId,
        destination: TerritoryId,
        arrival_event: EventId,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        let recipient = self
            .state
            .current
            .letters
            .get(&letter_id)
            .ok_or_else(|| CanwuError::new(ErrorCode::EntityNotFound, "arrival cargo disappeared"))?
            .recipient;
        let recipient_is_present =
            self.state
                .current
                .people
                .get(&recipient)
                .is_some_and(|person| {
                    person.current_location == destination && person.transit.is_none()
                });
        if recipient_is_present {
            self.deliver_letter(
                letter_id,
                carrier,
                destination,
                arrival_event,
                correlation_id,
            )
        } else {
            let letter = self
                .state
                .current
                .letters
                .get_mut(&letter_id)
                .ok_or_else(|| {
                    CanwuError::new(ErrorCode::EntityNotFound, "arrival cargo disappeared")
                })?;
            letter.status = LetterStatus::HeldAtLocation;
            letter.carrier = None;
            letter.location = Some(destination);
            Ok(())
        }
    }

    fn deliver_letter(
        &mut self,
        letter_id: super::LetterId,
        carrier: PersonId,
        territory: TerritoryId,
        arrival_event: EventId,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        let letter = self
            .state
            .current
            .letters
            .get_mut(&letter_id)
            .ok_or_else(|| {
                CanwuError::new(ErrorCode::EntityNotFound, "delivery letter disappeared")
            })?;
        let sender = letter.sender;
        let recipient = letter.recipient;
        letter.status = LetterStatus::Delivered;
        letter.carrier = None;
        letter.location = Some(territory);
        letter.delivered_at = Some(self.state.scheduler.now);
        self.emit(
            LetterDelivered {
                letter: letter_id,
                carrier,
                recipient,
                territory,
            }
            .into_kind(),
            vec![
                EntityRef::Resource(super::ResourceId::new(letter_id.get())),
                EntityRef::Person(sender),
                EntityRef::Person(carrier),
                EntityRef::Person(recipient),
                EntityRef::Territory(territory),
            ],
            format!("Letter {letter_id} was delivered to person {recipient}"),
            Some(CauseRef::Event(arrival_event)),
            correlation_id,
        )?;
        Ok(())
    }

    fn update_army_knowledge(
        &mut self,
        recipient: PersonId,
        army: ArmyId,
        location: TerritoryId,
        observed_at: SimTime,
        source: KnowledgeSource,
        confidence_per_mille: u16,
    ) {
        self.invalidate_commitments(CommitmentDomains::KNOWLEDGE);
        let (strength, known_name) = self.state.current.armies.get(&army).map_or_else(
            || (0, None),
            |value| (value.strength, Some(value.name.clone())),
        );
        let actor = self
            .state
            .current
            .knowledge
            .actors
            .entry(recipient)
            .or_insert_with(|| ActorKnowledge {
                actor: recipient,
                armies: BTreeMap::new(),
            });
        actor.armies.insert(
            army,
            ArmyKnowledge {
                army,
                known_name,
                known_location: Some(location),
                estimated_strength: EstimateRange {
                    minimum: strength.saturating_mul(9) / 10,
                    maximum: strength.saturating_mul(11) / 10,
                },
                observed_at,
                learned_at: self.state.scheduler.now,
                confidence_per_mille,
                source,
            },
        );
    }

    pub(super) fn emit(
        &mut self,
        kind: EventKind,
        affected_entities: Vec<EntityRef>,
        summary: String,
        cause: Option<CauseRef>,
        correlation_id: u64,
    ) -> Result<EventId, CanwuError> {
        let previous_depth = self.sync_reaction_depth;
        if previous_depth >= super::MAX_SYNCHRONOUS_REACTION_DEPTH {
            return Err(CanwuError::new(
                ErrorCode::SynchronousReactionLimit,
                format!(
                    "synchronous event reactors exceeded the maximum nested depth of {}",
                    super::MAX_SYNCHRONOUS_REACTION_DEPTH
                ),
            ));
        }
        self.sync_reaction_depth = previous_depth + 1;
        let result = self.emit_immediate(kind, affected_entities, summary, cause, correlation_id);
        self.sync_reaction_depth = previous_depth;
        result
    }

    fn emit_immediate(
        &mut self,
        kind: EventKind,
        affected_entities: Vec<EntityRef>,
        summary: String,
        cause: Option<CauseRef>,
        correlation_id: u64,
    ) -> Result<EventId, CanwuError> {
        let event = self.append_event(kind, affected_entities, summary, cause, correlation_id)?;
        let id = event.id;

        let systems = self.plugins.systems.clone();
        for registered in systems {
            let reader = format!("{}.{}", registered.plugin, registered.contract.name);
            let directives = catch_unwind(AssertUnwindSafe(|| {
                (registered.handler)(
                    &self.plugin_view(&reader, &registered.contract.reads),
                    &event,
                )
            }))
            .map_err(|_| {
                CanwuError::new(
                    ErrorCode::PluginPanicked,
                    format!(
                        "plugin system {}.{} panicked",
                        registered.plugin, registered.contract.name
                    ),
                )
            })??;
            validate_directives_with_context(
                &RuntimeValidationContext::new(&self.state),
                &registered.plugin,
                &registered.contract.writes,
                &self.plugins.state_owners,
                &self.plugins.record_schemas,
                &directives,
            )?;
            self.apply_directives(
                &registered.plugin,
                directives,
                &registered.contract.writes,
                &CauseRef::Event(id),
                correlation_id,
            )?;
        }
        Ok(id)
    }

    pub(super) fn append_event(
        &mut self,
        kind: EventKind,
        affected_entities: Vec<EntityRef>,
        summary: String,
        cause: Option<CauseRef>,
        correlation_id: u64,
    ) -> Result<SimEvent, CanwuError> {
        let (event_id, next_event_id) =
            claim_counter(self.state.counters.next_event_id, "event ID")?;
        let id = EventId::new(event_id);
        self.state.counters.next_event_id = next_event_id;
        let event = SimEvent {
            id,
            timestamp: self.state.scheduler.now,
            kind,
            affected_entities,
            summary,
            cause,
            correlation_id,
        };
        self.state.evidence.events.push(event.clone());
        Ok(event)
    }

    fn draw_random(
        &mut self,
        stream: &RandomStreamKey,
        upper_exclusive: u64,
        purpose: &str,
        producer: RandomDrawProducer,
        cause: CauseRef,
        correlation_id: u64,
    ) -> Result<(RandomDrawId, u64), CanwuError> {
        if upper_exclusive == 0
            || purpose.trim().is_empty()
            || purpose != purpose.trim()
            || correlation_id == 0
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draws require a positive bound, canonical purpose, and correlation",
            ));
        }
        let (draw_id, next_random_draw_id) =
            claim_counter(self.state.counters.next_random_draw_id, "random draw ID")?;
        self.invalidate_commitments(CommitmentDomains::RANDOM_STREAMS);
        let state = self
            .state
            .current
            .random_streams
            .get_mut(stream)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidRandomStream,
                    format!(
                        "random stream {}.{}@{} is not initialized",
                        stream.namespace, stream.name, stream.version
                    ),
                )
            })?;
        let next_position = state.position.checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "random stream position is exhausted",
            )
        })?;
        let position = state.position;
        let mut generator = DeterministicRng::from_seed(state.generator_state);
        let value = generator.range(upper_exclusive);
        state.position = next_position;
        state.generator_state = generator.state();
        self.state.counters.next_random_draw_id = next_random_draw_id;
        let id = RandomDrawId::new(draw_id);
        self.state.evidence.random_draws.push(RandomDrawRecord {
            id,
            at: self.state.scheduler.now,
            stream: stream.clone(),
            address: RandomDrawAddress::Sequential { position },
            operation_evidence: None,
            upper_exclusive,
            value,
            purpose: purpose.to_owned(),
            producer,
            outcome: None,
            cause,
            correlation_id,
        });
        Ok((id, value))
    }

    fn record_random_outcome(
        &mut self,
        id: RandomDrawId,
        outcome: RandomDrawOutcome,
    ) -> Result<(), CanwuError> {
        let Some(draw) = self
            .state
            .evidence
            .random_draws
            .last_mut()
            .filter(|draw| draw.id == id)
        else {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draw outcome does not match the latest pending draw",
            ));
        };
        if draw.outcome.replace(outcome).is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draw outcome was already recorded",
            ));
        }
        Ok(())
    }

    pub(super) fn append_boundary_random_draws(
        &mut self,
        boundary: BoundaryId,
        correlation_id: u64,
        draws: Vec<PendingBoundaryRandomDraw>,
    ) -> Result<Vec<RandomDrawId>, CanwuError> {
        let mut ids = Vec::with_capacity(draws.len());
        for pending in draws {
            let (draw_id, next_random_draw_id) =
                claim_counter(self.state.counters.next_random_draw_id, "random draw ID")?;
            let id = RandomDrawId::new(draw_id);
            self.state.counters.next_random_draw_id = next_random_draw_id;
            self.state.evidence.random_draws.push(RandomDrawRecord {
                id,
                at: self.state.scheduler.now,
                stream: pending.draw.stream,
                address: pending.draw.address,
                operation_evidence: pending.draw.operation_evidence,
                upper_exclusive: pending.draw.upper_exclusive,
                value: pending.draw.value,
                purpose: pending.draw.purpose,
                producer: RandomDrawProducer::BoundarySystem {
                    boundary,
                    plugin: pending.plugin,
                    system: pending.system,
                },
                outcome: Some(RandomDrawOutcome::BoundarySystemDecision),
                cause: CauseRef::Boundary(boundary),
                correlation_id,
            });
            ids.push(id);
        }
        Ok(ids)
    }

    pub(super) fn schedule_at(
        &mut self,
        at: SimTime,
        action: ScheduledAction,
    ) -> Result<(), CanwuError> {
        if at <= self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "scheduled work must target a strictly future simulation time",
            ));
        }
        let (sequence, next_sequence) = claim_counter(
            self.state.counters.next_schedule_sequence,
            "schedule sequence",
        )?;
        let key = ScheduleKey { at, sequence };
        self.state.counters.next_schedule_sequence = next_sequence;
        self.invalidate_commitments(CommitmentDomains::SCHEDULER);
        if self.state.scheduler.actions.insert(key, action).is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "the runtime attempted to reuse a schedule key",
            ));
        }
        Ok(())
    }

    pub(super) fn plugin_view<'a>(
        &'a self,
        reader: &'a str,
        reads: &'a [StateKey],
    ) -> SimulationView<'a> {
        SimulationView {
            state: SimulationViewState::Runtime(&self.state),
            state_owners: &self.plugins.state_owners,
            reader: Some(reader),
            allowed_reads: Some(reads),
            allowed_ingress: None,
            ingress_plugin: None,
            component_overlay: None,
            proposed_components: None,
            record_overlay: None,
            proposed_records: None,
            boundary_id: None,
            proposal_evidence: None,
            knowledge_overlay: None,
            allocations: None,
            allowed_reservations: None,
            random_session: None,
        }
    }
}
