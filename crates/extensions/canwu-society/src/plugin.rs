use crate::ingress::{
    COHORT_REBASE_INGRESS, CohortHeadcountRebaseV1, MAX_SOCIETY_INGRESS_QUEUE,
    QueuedSocietyIngress, SOCIETY_INGRESS_MALFORMED_REJECTION,
    SOCIETY_INGRESS_QUEUE_FULL_REJECTION, SocietyIngressBatch, SocietyIngressBatchRecord,
    SocietyIngressQueue, SocietyIngressQueueRecord, is_queued_packet, queue_entries,
    rebase_admission_rejection, society_ingress_batch_reference, society_ingress_queue_reference,
};
use crate::intergroup::{
    INTERGROUP_RELATION_INGRESS, INTERGROUP_RELATION_REJECTED_EVENT, IntergroupRelationRevisionV1,
    IntergroupRelations, IntergroupRelationsRecord, cohort_classes, intergroup_relations_reference,
    is_region_kind,
};
use crate::lifecycle::{SOCIETY_LIFECYCLE_DELTA_INGRESS, SocietyLifecycleDeltaV1};
use crate::model::{
    CohortHeadcountRebaseOutcome, CohortTransferIntent, CohortTransferOutcome, DispositionBucket,
    DispositionDistribution, DispositionProfile, InstitutionalAlignment, PendingCohortTransfer,
    PolicyDecision, SocietyCohort, SocietyCohortExchangeLedger, SocietyCohortExchangeLedgerRecord,
    SocietyCohortTransferPending, SocietyCohortTransferPendingRecord, SocietyIngressStatus,
    SocietyLifecycleDeltaOutcome, SocietyProjection, SocietyState, SocietyStateRecord,
    core_reference_schemas, invalid, society_cohort_exchange_ledger_reference,
    society_cohort_transfer_pending_reference, society_state_reference,
};
use crate::revision::{
    PRESSURE_REVISION_AUTHORITY_REJECTION, PRESSURE_REVISION_COMMAND,
    PRESSURE_REVISION_ORIGIN_REJECTION, PRESSURE_REVISION_SCOPE_REJECTION,
    PRESSURE_REVISION_STALE_REJECTION, PRESSURE_REVISION_SUPERSEDED_REJECTION,
    PressureRevisionDecision, PressureRevisionOutcome, RevisionScopeError,
    SOCIETY_PRESSURE_REVISED_EVENT, SOCIETY_PRESSURE_REVISION_INGRESS,
    SOCIETY_PRESSURE_REVISION_REJECTED_EVENT, SocietyPressureRevisionV1,
    pressure_revision_admission_rejection,
};
use crate::solver::{
    compute_aggregates, compute_mobilization_candidates, compute_projections,
    settle_valid_transitions,
};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, CommandId, DecisionOrigin, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, EntityRef, ErrorCode, IngressClass,
    IngressPayload, Issuer, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginActionDescriptor, PluginIngressDescriptor, PluginRegistrar, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const PLUGIN_NAME: &str = "canwu-society";

/// The society plugin: the only writer of `canwu.society:state`.
///
/// # Settlement cadence
///
/// Society settles in the boundaries that carry the plugin's settlement
/// cadence, a [`SystemCadence`] category: `Daily` for [`Self::DAILY`] and
/// [`Self::default`], or the category given to
/// [`Self::with_settlement_cadence`]. Canwu keeps no calendar, so the host
/// decides which boundaries carry the category: a `Monthly` cadence settles
/// society whenever the host settles a `Monthly` boundary, whatever the
/// length of its months, and a host that wants a week or another declared
/// period schedules a category it does not otherwise use, for example
/// `EraScheduled`, at that period.
///
/// The settlement system `settle-social-transitions` (phase 7) runs on that
/// cadence. The event-driven intakes do not: the cohort-transfer and
/// owner-side queue intakes queue admitted work at any boundary for the next
/// settlement, and the intergroup relation intake applies an admitted
/// relation revision in the boundary that admits it (see
/// [`crate::IntergroupRelations`]), since the settlement does not use the
/// relations.
///
/// A transition rate is **per settlement**, not per unit of simulated time
/// (see [`crate::settle_transitions`]), so an application authors each rate
/// for the cadence it registers. The cadence is recorded in the plugin
/// descriptor that every snapshot persists: a run is restored and replayed
/// only with a plugin of the same cadence, and any other is refused with
/// `PluginManifestMismatch`.
///
/// Everything the phase-7 system applies waits for the cadence: due cohort
/// transfers, queued rebases, lifecycle deltas and pressure revisions,
/// institutional decisions, and transitions. Under a coarse cadence that
/// has two costs. Every issuer in the period decided against the same state,
/// so by default ([`TransferSettlement::Batch`]) the due transfers are all
/// checked against the state at the start of the settlement and each one
/// that still fits completes; [`Self::with_transfer_settlement`] with
/// [`TransferSettlement::OneByOne`] instead checks each against the state
/// the earlier transfers of the settlement left, so of the due transfers
/// that share a cohort one completes per settlement and the others settle as
/// `stale_source` and must be reissued. And at most
/// [`crate::MAX_SOCIETY_INGRESS_QUEUE`] packets wait: a further one is
/// refused by the boundary that admits it, with a terminal `queue_full`
/// outcome in the cohort exchange ledger, and a pressure revision command is
/// refused at admission while the queue is full. A refused rebase leaves its
/// cohort behind its stock until the provider sends another, and a refused
/// pressure revision must be decided again. The queue itself costs evidence
/// in proportion to what each boundary admits: a boundary writes its own
/// batch record and the queue's small head (see
/// [`crate::SocietyIngressQueue`]).
///
/// A settlement writes the society root at most once: it materializes, in
/// that write, each derived view its own changes cleared (see the `last_*_at`
/// fields of [`crate::SocietyState`]). Until 0.18.0 three further systems in
/// phases 10, 12, and 13 did that, each with its own write. A settlement
/// that changes nothing writes nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocietyPlugin {
    settlement_cadence: SystemCadence,
    transfer_settlement: TransferSettlement,
    outcome_retention: OutcomeRetention,
}

/// How long the cohort exchange ledger keeps the outcomes of queued rebases
/// and lifecycle deltas.
///
/// The retention is part of the plugin's identity, like the
/// [`TransferSettlement`] mode: a run is restored and replayed only with a
/// plugin of the retention it was registered with
/// (`PluginManifestMismatch` otherwise). Cohort transfer outcomes and the
/// latest pressure revision outcomes of each alignment are kept under every
/// retention: a transfer's outcome is what makes a resent transfer
/// idempotent, and the revision outcomes are already one or two per
/// alignment.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OutcomeRetention {
    /// Every rebase and lifecycle delta outcome stays in the ledger for the
    /// life of the run, so the ledger, which a settlement rewrites whole,
    /// grows with every packet. The default, and the rule of every release
    /// before 0.18.0.
    #[default]
    Keep,
    /// A settlement that records rebase or lifecycle delta outcomes first
    /// drops those recorded before it, so the ledger holds the outcomes of
    /// the latest settlement that recorded any, and of packets refused for a
    /// full queue since then. A host reads the outcomes before the next such
    /// settlement; the record-change evidence keeps the earlier ledger
    /// versions until the history is sealed.
    LatestSettlement,
}

/// How a society settlement checks the cohort transfers due at it.
///
/// The mode is part of the plugin's identity: the two modes have different
/// semantic hashes, so a run is restored and replayed only with a plugin of
/// the mode it was registered with (`PluginManifestMismatch` otherwise).
///
/// Choose [`Self::Batch`] when several transfers may move people out of one
/// cohort, or into it, between two settlements, which is the usual case under
/// a coarse settlement cadence: every issuer in the period decided against
/// the same state, and each transfer that still fits completes. Choose
/// [`Self::OneByOne`] when each transfer must see the result of the one
/// before it, so that a transfer decided before an earlier one moved its
/// cohorts is reissued against the new state rather than applied.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TransferSettlement {
    /// Each due transfer is checked against the state the earlier transfers
    /// of the same settlement left. Of the due transfers that share a
    /// cohort, as source or destination, the first in operation-ID order
    /// completes and the others settle as `stale_source` and must be
    /// reissued against the new state. The rule of every release before
    /// 0.16.0.
    OneByOne,
    /// Every due transfer is checked against the state at the start of the
    /// settlement, the state the issuers of the period could see: one whose
    /// cohorts changed since it was admitted settles as `stale_source`. The
    /// others apply in admission order, each to the state the earlier ones
    /// left, so conservation holds; one whose quantity no longer fits its
    /// source cohort settles as
    /// [`COHORT_TRANSFER_INSUFFICIENT_SOURCE`], a recorded outcome, and
    /// never fails the boundary. The default.
    #[default]
    Batch,
}

impl SocietyPlugin {
    /// The society plugin on the default `Daily` settlement cadence, with
    /// the default [`TransferSettlement::Batch`] transfers.
    pub const DAILY: Self = Self {
        settlement_cadence: SystemCadence::Daily,
        transfer_settlement: TransferSettlement::Batch,
        outcome_retention: OutcomeRetention::Keep,
    };

    /// Returns the society plugin whose settlement system runs on `cadence`,
    /// with the default [`TransferSettlement::Batch`] transfers and
    /// [`OutcomeRetention::Keep`].
    ///
    /// Registration fails with `InvalidPluginRegistration` for
    /// `SystemCadence::EventDriven`, which is not a category a host
    /// schedules.
    #[must_use]
    pub const fn with_settlement_cadence(cadence: SystemCadence) -> Self {
        Self {
            settlement_cadence: cadence,
            transfer_settlement: TransferSettlement::Batch,
            outcome_retention: OutcomeRetention::Keep,
        }
    }

    /// Returns this plugin with the cohort exchange ledger keeping rebase and
    /// lifecycle delta outcomes as `retention` says; see
    /// [`OutcomeRetention`].
    #[must_use]
    pub fn with_outcome_retention(self, retention: OutcomeRetention) -> Self {
        Self {
            outcome_retention: retention,
            ..self
        }
    }

    /// How long the ledger keeps rebase and lifecycle delta outcomes.
    #[must_use]
    pub const fn outcome_retention(&self) -> OutcomeRetention {
        self.outcome_retention
    }

    /// Returns this plugin with due cohort transfers settled in `mode`; see
    /// [`TransferSettlement`] for when to choose which.
    #[must_use]
    pub fn with_transfer_settlement(self, mode: TransferSettlement) -> Self {
        Self {
            transfer_settlement: mode,
            ..self
        }
    }

    /// The cadence category whose boundaries settle society.
    #[must_use]
    pub fn settlement_cadence(&self) -> SystemCadence {
        self.settlement_cadence.clone()
    }

    /// How a settlement checks the cohort transfers due at it.
    #[must_use]
    pub const fn transfer_settlement(&self) -> TransferSettlement {
        self.transfer_settlement
    }
}

impl Default for SocietyPlugin {
    fn default() -> Self {
        Self::DAILY
    }
}

impl SimulationPlugin for SocietyPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        // The handlers are the same for every settlement cadence; the cadence
        // itself is part of the descriptor's boundary-system contracts. The
        // transfer settlement mode and the outcome retention change what the
        // settlement handler does, so each pair has its own hash.
        match (self.transfer_settlement, self.outcome_retention) {
            (TransferSettlement::OneByOne, OutcomeRetention::Keep) => {
                "1e22be0ca947972377b255ae265b8fcdae388a280d3ac9eecd76a377445def0b"
            }
            (TransferSettlement::OneByOne, OutcomeRetention::LatestSettlement) => {
                "e806c8d8059d55761ce8414edc2947e35c921afadf287fd73c7c5a30b98b19d9"
            }
            (TransferSettlement::Batch, OutcomeRetention::Keep) => {
                "4cda4b31ad7c992b9b98d434fec07a87313e3ee5f95d1e9b8d04d37efb4863cc"
            }
            (TransferSettlement::Batch, OutcomeRetention::LatestSettlement) => {
                "021a578a83db331be2f08d91496bd45b710b77bc5276dc5dea065696006efecb"
            }
        }
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        let batch_kind = canwu_api::DomainRecordKind::for_type::<SocietyIngressBatchRecord>();
        let mut head = SocietyIngressQueue::empty();
        let mut batches = BTreeMap::new();
        let mut relations = None;
        let mut root = None;
        for record in records {
            if record.reference == society_ingress_queue_reference().into_untyped() {
                head = record.decode_payload::<SocietyIngressQueueRecord>()?;
                head.validate()?;
            } else if record.reference.kind == batch_kind {
                let slot = batch_slot(&record.reference.id)?;
                if record.reference != society_ingress_batch_reference(slot).into_untyped() {
                    return Err(invalid("a society ingress batch ID is not its slot"));
                }
                batches.insert(slot, record.decode_payload::<SocietyIngressBatchRecord>()?);
            } else if record.reference == society_cohort_exchange_ledger_reference().into_untyped()
            {
                record
                    .decode_payload::<SocietyCohortExchangeLedgerRecord>()?
                    .validate()?;
            } else if record.reference == intergroup_relations_reference().into_untyped() {
                relations = Some(record);
            } else if record.reference == society_state_reference().into_untyped() {
                root = Some(record);
            }
        }
        // Authored intergroup relations name classes some authored cohort
        // carries; cohorts and their classifications never change at
        // runtime, so the check holds for the whole run.
        if let Some(record) = relations {
            let relations = decode_relations(record)?;
            let classes = match root {
                Some(root) => root
                    .decode_payload::<SocietyStateRecord>()?
                    .cohort_classes(),
                None => BTreeSet::new(),
            };
            relations.validate_classes(&classes)?;
        }
        // The slots in use hold the queue the head counts, in admission
        // order; every other slot is empty, waiting to be reused.
        let in_use = (0..head.batches)
            .map(|slot| {
                batches
                    .remove(&slot)
                    .ok_or_else(|| invalid("a society ingress batch in use is missing"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        queue_entries(&head, in_use)?;
        for batch in batches.values() {
            batch.validate()?;
            if !batch.entries.is_empty() {
                return Err(invalid(
                    "a society ingress batch that is not in use holds packets",
                ));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let cadence = || self.settlement_cadence();
        if cadence() == SystemCadence::EventDriven {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "the society settlement cadence must be a scheduled cadence category",
            ));
        }
        let mut schema = DomainRecordSchema::for_record::<SocietyStateRecord>();
        schema.payload_schema = society_payload_schema();
        schema.references = core_reference_schemas();
        registrar.register_record_schema(schema)?;

        let mut ledger_schema =
            DomainRecordSchema::for_record::<SocietyCohortExchangeLedgerRecord>();
        ledger_schema.payload_schema = exchange_ledger_payload_schema();
        registrar.register_record_schema(ledger_schema)?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            SocietyCohortTransferPendingRecord,
        >())?;
        let mut queue_schema = DomainRecordSchema::for_record::<SocietyIngressQueueRecord>();
        queue_schema.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "schema_version".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("queued".to_owned(), required(PayloadValueType::Integer)),
                ("batches".to_owned(), required(PayloadValueType::Integer)),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(queue_schema)?;
        let mut batch_schema = DomainRecordSchema::for_record::<SocietyIngressBatchRecord>();
        batch_schema.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "schema_version".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("entries".to_owned(), required(PayloadValueType::Array)),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(batch_schema)?;
        let mut relations_schema = DomainRecordSchema::for_record::<IntergroupRelationsRecord>();
        relations_schema.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "schema_version".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("kinds".to_owned(), optional(PayloadValueType::Array)),
                ("edges".to_owned(), optional(PayloadValueType::Object)),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(relations_schema)?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: INTERGROUP_RELATION_INGRESS.to_owned(),
            description: "Create or revise intergroup relations between cohort classes".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([(
                    "changes".to_owned(),
                    required(PayloadValueType::Array),
                )]),
                allow_additional: false,
            },
        })?;
        let mut relation_intake = BoundarySystemContract::new(
            "intake-intergroup-relations",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        relation_intake.reads = vec![
            intergroup_relations_key(),
            society_state_key(),
            StateKey::core_domain_records(),
            StateKey::core_governments(),
            StateKey::core_ingress(),
            StateKey::core_territories(),
        ];
        relation_intake.reads.sort();
        relation_intake.writes = vec![intergroup_relations_key()];
        relation_intake.emits = vec![INTERGROUP_RELATION_REJECTED_EVENT.to_owned()];
        relation_intake.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(relation_intake, intake_intergroup_relations)?;

        registrar.register_ingress(PluginIngressDescriptor {
            name: COHORT_REBASE_INGRESS.to_owned(),
            description: "Rebase one cohort headcount to an exact external stock version"
                .to_owned(),
            class: IngressClass::Information,
            payload_schema: rebase_payload_schema(),
        })?;
        // Only boundary systems that declare this target may schedule a
        // lifecycle delta; the host cannot author one.
        let _lifecycle_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: SOCIETY_LIFECYCLE_DELTA_INGRESS.to_owned(),
            description: "Apply one provider-computed target lifecycle delta".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([
                    ("installs".to_owned(), optional(PayloadValueType::Array)),
                    (
                        "deactivations".to_owned(),
                        optional(PayloadValueType::Array),
                    ),
                    ("releases".to_owned(), optional(PayloadValueType::Array)),
                ]),
                allow_additional: false,
            },
        })?;
        // Only the revision command queues an admissible pressure revision;
        // the host cannot author one.
        let _revision_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: SOCIETY_PRESSURE_REVISION_INGRESS.to_owned(),
            description: "Apply one authority-checked pressure revision decision".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([
                    ("decision".to_owned(), required(PayloadValueType::Object)),
                    ("issuer".to_owned(), required(PayloadValueType::Object)),
                    ("command".to_owned(), required(PayloadValueType::Integer)),
                ]),
                allow_additional: false,
            },
        })?;

        let mut ingress_intake = BoundarySystemContract::new(
            "intake-society-ingress",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        ingress_intake.reads = vec![
            ingress_queue_key(),
            ingress_batch_key(),
            cohort_exchange_ledger_key(),
            StateKey::core_domain_records(),
            StateKey::core_ingress(),
        ];
        ingress_intake.reads.sort();
        ingress_intake.writes = vec![
            ingress_queue_key(),
            ingress_batch_key(),
            cohort_exchange_ledger_key(),
        ];
        ingress_intake.writes.sort();
        ingress_intake.emits = vec![
            SOCIETY_INGRESS_REJECTED_EVENT.to_owned(),
            SOCIETY_PRESSURE_REVISION_REJECTED_EVENT.to_owned(),
        ];
        ingress_intake.emits.sort();
        ingress_intake.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(ingress_intake, intake_society_ingress)?;

        registrar.register_command(
            PluginActionDescriptor {
                name: "transfer_cohort_population".to_owned(),
                description: "Execute an authority-checked owner-side cohort population transfer"
                    .to_owned(),
                payload_schema: cohort_transfer_payload_schema(),
                reads: vec![
                    society_state_key(),
                    cohort_exchange_ledger_key(),
                    pending_transfer_key(),
                ],
                writes: Vec::new(),
            },
            transfer_cohort_population,
        )?;

        let mut transfer_intake = BoundarySystemContract::new(
            "intake-cohort-transfers",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        transfer_intake.reads = vec![
            society_state_key(),
            cohort_exchange_ledger_key(),
            pending_transfer_key(),
            StateKey::core_ingress(),
        ];
        transfer_intake.reads.sort();
        transfer_intake.writes = vec![pending_transfer_key()];
        transfer_intake.emits = vec![SOCIETY_INGRESS_REJECTED_EVENT.to_owned()];
        transfer_intake.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(transfer_intake, intake_cohort_transfer)?;

        registrar.register_ingress(PluginIngressDescriptor {
            name: COHORT_TRANSFER_INGRESS.to_owned(),
            description: "Apply an admitted cohort transfer".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "set_institutional_policy".to_owned(),
                description: "Record a versioned institutional alignment decision".to_owned(),
                payload_schema: policy_payload_schema(),
                reads: vec![society_state_key(), policy_decision_key()],
                writes: vec![policy_decision_key()],
            },
            set_institutional_policy,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: PRESSURE_REVISION_COMMAND.to_owned(),
                description: "Queue a versioned institutional revision of policy pressure and \
                              of influence edge and organization activity and reach"
                    .to_owned(),
                payload_schema: pressure_revision_payload_schema(),
                reads: vec![
                    society_state_key(),
                    cohort_exchange_ledger_key(),
                    ingress_queue_key(),
                ],
                writes: Vec::new(),
            },
            revise_society_pressure,
        )?;

        let mut transition = BoundarySystemContract::new(
            "settle-social-transitions",
            BoundaryPhase::DomainDeltaProposal,
            cadence(),
        );
        transition.reads = vec![
            society_state_key(),
            policy_decision_key(),
            cohort_exchange_ledger_key(),
            pending_transfer_key(),
            ingress_queue_key(),
            ingress_batch_key(),
            StateKey::core_ingress(),
        ];
        transition.writes = vec![
            society_state_key(),
            cohort_exchange_ledger_key(),
            ingress_queue_key(),
            ingress_batch_key(),
        ];
        transition.emits = vec![
            SOCIETY_PRESSURE_REVISED_EVENT.to_owned(),
            SOCIETY_PRESSURE_REVISION_REJECTED_EVENT.to_owned(),
        ];
        transition.visibility = StateVisibility::SameBoundary;
        // The one settlement system: it applies everything that waits for the
        // cadence and materializes the derived views in the same write, so a
        // settlement writes the society root at most once.
        let settle: canwu_api::BoundarySystemHandler =
            match (self.transfer_settlement, self.outcome_retention) {
                (TransferSettlement::OneByOne, OutcomeRetention::Keep) => {
                    settle_society_one_by_one_keep
                }
                (TransferSettlement::OneByOne, OutcomeRetention::LatestSettlement) => {
                    settle_society_one_by_one_latest
                }
                (TransferSettlement::Batch, OutcomeRetention::Keep) => settle_society_batch_keep,
                (TransferSettlement::Batch, OutcomeRetention::LatestSettlement) => {
                    settle_society_batch_latest
                }
            };
        registrar.register_boundary_system(transition, settle)
    }
}

/// Admits one cohort transfer and queues it for its due time.
///
/// Every expected refusal uses an error code the engine records as a
/// rejected command attempt, so a refused transfer never fails its boundary
/// or blocks the commands queued behind it: `InvalidPayload` for a malformed
/// intent, `InvalidAuthority` for a missing or mismatched authority (a run
/// without a society root has no alignment and so no authority),
/// `EntityNotFound` for an unknown source or destination cohort,
/// `InvalidDuration` for a due time in the past, `ValueOutOfRange` for an
/// expected source version that is not the root's current version, and
/// `IdempotencyConflict` for an operation ID bound to another settled or
/// pending transfer. A resend of a settled or pending transfer is accepted
/// and queues nothing. An invalid society record is not a refusal and still
/// fails the boundary.
fn transfer_cohort_population(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let intent: CohortTransferIntent =
        serde_json::from_value(payload.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("cohort transfer intent could not be decoded: {error}"),
            )
        })?;
    validate_transfer_intent(&intent)?;
    let Some((record, state)) = load_state(view)? else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "the society state record is not configured, so no alignment grants authority",
        ));
    };
    let alignment = state
        .institutional_alignments
        .get(&intent.authority_alignment_id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "unknown transfer authority alignment",
            )
        })?;
    let actor = match (&context.issuer, &context.authority.decision_origin) {
        (Issuer::Actor(issuer), DecisionOrigin::Actor { actor })
            if issuer == actor && Some(*actor) == alignment.authorized_actor =>
        {
            *actor
        }
        _ => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "cohort transfer requires the authorized actor ingress",
            ));
        }
    };
    if context.authority.command_subject.as_ref() != Some(&alignment.institution) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "transfer command subject does not own the authority alignment",
        ));
    }
    for cohort_id in [&intent.source_cohort_id, &intent.destination_cohort_id] {
        if !state.cohorts.contains_key(cohort_id) {
            return Err(CanwuError::new(
                ErrorCode::EntityNotFound,
                format!("cohort transfer names unknown cohort {cohort_id}"),
            ));
        }
    }
    if intent.due_time < view.time() {
        return Err(CanwuError::new(
            ErrorCode::InvalidDuration,
            "cohort transfer due time is stale",
        ));
    }
    let (ledger, _) = load_ledger(view)?;
    if let Some(existing) = ledger.outcomes.get(&intent.operation_id) {
        if existing.source_record_version == intent.expected_source_version
            && existing.source_cohort_id == intent.source_cohort_id
            && existing.destination_cohort_id == intent.destination_cohort_id
            && existing.quantity == intent.quantity
        {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "operation id is already bound to a different transfer",
        ));
    }
    // A transfer that was admitted and has not settled yet holds its
    // operation ID as well. One whose packet is still in flight is not
    // visible here; the intake settles that case.
    if let Some(existing) = load_pending(view)?.0.transfers.get(&intent.operation_id) {
        if existing.intent == intent {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "operation id is already bound to a different pending transfer",
        ));
    }
    if record.version != intent.expected_source_version {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            format!(
                "society record version {} does not match expected {}",
                record.version, intent.expected_source_version
            ),
        ));
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::days(1),
        packet_type: COHORT_TRANSFER_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::json!({"intent": intent, "actor": actor.get(), "source_record_version": record.version, "command_id": context.command_id.get()}),
        affected: vec![alignment.institution.clone()],
    }])
}

const COHORT_TRANSFER_INGRESS: &str = "cohort-transfer";
/// Event a society intake emits for an admitted packet it does not queue:
/// a rebase or lifecycle delta that arrives while the ingress queue is full,
/// whose `queue_full` outcome the ledger also records, and a cohort transfer
/// packet that is malformed, was not produced by the transfer command, names
/// an unknown cohort, has lost its authority, or reuses an operation ID with
/// other data. The summary names the ingress and ends with the reason; the
/// boundary commits. A pressure revision refused for a full queue is
/// reported as `society_pressure_revision_rejected_v1`, as every rejected
/// pressure revision is.
pub const SOCIETY_INGRESS_REJECTED_EVENT: &str = "society_ingress_rejected_v1";
const TRANSFER_UNKNOWN_COHORT: &str = "unknown_cohort";
const TRANSFER_OPERATION_CONFLICT: &str = "operation_id_conflict";
const TRANSFER_COMPLETED: &str = "completed";
const TRANSFER_STALE_SOURCE: &str = "stale_source";
const TRANSFER_REJECTED: &str = "rejected";
/// The `result` of a cohort transfer outcome that settled in a
/// [`TransferSettlement::Batch`] settlement after the earlier transfers of
/// that settlement left its source cohort with no more people than it moves:
/// a terminal rejection, recorded in the cohort exchange ledger. A transfer
/// always leaves its source at least one person.
pub const COHORT_TRANSFER_INSUFFICIENT_SOURCE: &str = "insufficient_source";

fn pending_transfer_key() -> StateKey {
    StateKey::new(
        SocietyCohortTransferPendingRecord::NAMESPACE,
        SocietyCohortTransferPendingRecord::NAME,
    )
}

/// Queues every admitted cohort transfer packet in the pending record.
///
/// The system is total over what a packet can carry: a defective packet is
/// skipped with a [`SOCIETY_INGRESS_REJECTED_EVENT`] and the boundary
/// commits. Only an invalid society root, ledger, or pending record is an
/// error.
///
/// A boundary that queues a transfer rewrites the pending record without the
/// transfers the ledger already records as settled, so the record holds the
/// transfers in flight and those settled since the last intake, not every
/// transfer of the run. A settled transfer needs no pending entry: its
/// ledger outcome is what makes a resend or a second delivery a no-op, here
/// and in the command, and what makes the settlement skip it.
fn intake_cohort_transfer(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    // This system runs in every boundary that admits an event or ingress of
    // any plugin; it reads society records only when one of them is a
    // cohort transfer.
    let mut packets = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && matches!(
                &ingress.payload,
                IngressPayload::Plugin { plugin, packet_type, .. }
                    if plugin == PLUGIN_NAME && packet_type == COHORT_TRANSFER_INGRESS
            )
        {
            packets.push(ingress);
        }
    }
    if packets.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    // The intake reads two cohorts and one alignment of the committed root,
    // and the ledger outcomes of the operations it sees, which their owner
    // validated when it wrote them, so it decodes neither record whole.
    let root = view
        .typed_domain_record(&society_state_reference())?
        .map(|record| &record.payload);
    let outcomes = view
        .typed_domain_record(&society_cohort_exchange_ledger_reference())?
        .and_then(|record| record.payload.get("outcomes"));
    let (mut pending, pending_record) = load_pending(view)?;
    let mut directives = Vec::new();
    let mut changed = false;
    for ingress in packets {
        match admitted_cohort_transfer(root, outcomes, &pending, ingress)? {
            Ok(Some(transfer)) => {
                pending
                    .transfers
                    .insert(transfer.intent.operation_id.clone(), transfer);
                changed = true;
            }
            // A second delivery of a transfer that is already pending or
            // settled.
            Ok(None) => {}
            Err(reason) => directives.push(BoundaryDirective::Emit {
                event_type: SOCIETY_INGRESS_REJECTED_EVENT.to_owned(),
                summary: format!(
                    "Rejected cohort transfer ingress {}: {reason}",
                    ingress.id.get()
                ),
                affected: Vec::new(),
            }),
        }
    }
    if changed {
        pending.transfers.retain(|operation_id, _| {
            outcomes.is_none_or(|outcomes| outcomes.get(operation_id).is_none())
        });
        directives.push(record_write(
            society_cohort_transfer_pending_reference(),
            &pending,
            pending_record.as_ref(),
            "Persist admitted society cohort transfer",
        )?);
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Decides one admitted cohort transfer packet: the pending transfer to
/// queue, `None` for a second delivery of a pending or settled one, or the
/// reason the packet is rejected.
///
/// A packet is authentic exactly when its ingress record names, as its
/// cause, the command its payload carries. Only the transfer command's
/// handler produces that, after checking the issuer's authority; a host
/// cannot give a packet a command cause. The check reads the ingress record
/// in the boundary that admits it and no retained command evidence, so a
/// sealed run and its exact replay decide alike. Authority is then checked
/// again against the current root: the alignment must still authorize the
/// actor for the institution the packet was queued for.
fn admitted_cohort_transfer(
    root: Option<&Value>,
    outcomes: Option<&Value>,
    pending: &SocietyCohortTransferPending,
    ingress: &canwu_api::IngressRecord,
) -> Result<Result<Option<PendingCohortTransfer>, &'static str>, CanwuError> {
    let IngressPayload::Plugin {
        payload,
        affected_entities,
        ..
    } = &ingress.payload
    else {
        return Ok(Err(SOCIETY_INGRESS_MALFORMED_REJECTION));
    };
    let (Some(intent), Some(actor), Some(command_id)) = (
        payload
            .get("intent")
            .and_then(|value| serde_json::from_value::<CohortTransferIntent>(value.clone()).ok())
            .filter(|intent| validate_transfer_intent(intent).is_ok()),
        payload.get("actor").and_then(Value::as_u64),
        payload.get("command_id").and_then(Value::as_u64),
    ) else {
        return Ok(Err(SOCIETY_INGRESS_MALFORMED_REJECTION));
    };
    if ingress.cause != Some(canwu_api::CauseRef::Command(CommandId::new(command_id))) {
        return Ok(Err(PRESSURE_REVISION_ORIGIN_REJECTION));
    }
    let actor = canwu_api::PersonId::new(actor);
    let Some(root) = root else {
        return Ok(Err(PRESSURE_REVISION_AUTHORITY_REJECTION));
    };
    if !record_entry::<InstitutionalAlignment>(
        root.get("institutional_alignments"),
        &intent.authority_alignment_id,
    )?
    .is_some_and(|alignment| {
        alignment.authorized_actor == Some(actor)
            && affected_entities.contains(&alignment.institution)
    }) {
        return Ok(Err(PRESSURE_REVISION_AUTHORITY_REJECTION));
    }
    let (Some(source), Some(destination)) = (
        record_entry::<SocietyCohort>(root.get("cohorts"), &intent.source_cohort_id)?,
        record_entry::<SocietyCohort>(root.get("cohorts"), &intent.destination_cohort_id)?,
    ) else {
        return Ok(Err(TRANSFER_UNKNOWN_COHORT));
    };
    let transfer = PendingCohortTransfer {
        source_digest: cohort_pair_digest(&source, &destination)?,
        intent,
        actor,
        command_id,
    };
    let operation_id = &transfer.intent.operation_id;
    // A settled transfer's outcome records its whole intent and its actor.
    if let Some(settled) = record_entry::<CohortTransferOutcome>(outcomes, operation_id)? {
        let intent = &transfer.intent;
        let same = settled.source_record_version == intent.expected_source_version
            && settled.source_cohort_id == intent.source_cohort_id
            && settled.destination_cohort_id == intent.destination_cohort_id
            && settled.quantity == intent.quantity
            && settled.authority_alignment_id == intent.authority_alignment_id
            && settled.due_time == intent.due_time
            && settled.actor == actor;
        return Ok(if same {
            Ok(None)
        } else {
            Err(TRANSFER_OPERATION_CONFLICT)
        });
    }
    Ok(match pending.transfers.get(operation_id) {
        None => Ok(Some(transfer)),
        // The same transfer delivered again, by the same or a resent command.
        Some(existing) if existing.intent == transfer.intent && existing.actor == actor => Ok(None),
        Some(_) => Err(TRANSFER_OPERATION_CONFLICT),
    })
}

/// One entry of a map of a committed society record, decoded alone.
fn record_entry<T: serde::de::DeserializeOwned>(
    map: Option<&Value>,
    id: &str,
) -> Result<Option<T>, CanwuError> {
    map.and_then(|entries| entries.get(id))
        .map(|entry| {
            T::deserialize(entry).map_err(|error| {
                invalid(format!("society record entry {id} is malformed: {error}"))
            })
        })
        .transpose()
}

fn load_pending(
    view: &SimulationView<'_>,
) -> Result<(SocietyCohortTransferPending, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&society_cohort_transfer_pending_reference())?
    else {
        return Ok((
            SocietyCohortTransferPending {
                schema_version: 1,
                transfers: BTreeMap::new(),
            },
            None,
        ));
    };
    let pending = record.decode_payload::<SocietyCohortTransferPendingRecord>()?;
    if pending.schema_version != 1 {
        return Err(invalid("unsupported pending cohort transfer schema"));
    }
    Ok((pending, Some(record.clone())))
}

fn cohort_pair_digest(
    source: &SocietyCohort,
    destination: &SocietyCohort,
) -> Result<String, CanwuError> {
    // Bind only the cohorts the transfer moves between. Their dispositions
    // may keep changing through ordinary transitions; the transfer moves the
    // proportional share current when it settles.
    canwu_api::canonical_hash(
        "canwu.society.cohort-transfer-source.v2",
        &(source, destination),
    )
}

fn validate_transfer_intent(intent: &CohortTransferIntent) -> Result<(), CanwuError> {
    if intent.operation_id.is_empty()
        || intent.authority_alignment_id.is_empty()
        || intent.source_cohort_id.is_empty()
        || intent.destination_cohort_id.is_empty()
        || intent.quantity == 0
        || intent.expected_source_version == 0
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "cohort transfer intent has empty or zero required fields",
        ));
    }
    if intent.source_cohort_id == intent.destination_cohort_id {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "source and destination cohorts must differ",
        ));
    }
    Ok(())
}

/// Moves one transfer's people, every distribution's proportional share,
/// from its source cohort to its destination, or changes nothing and returns
/// an error.
///
/// It reads and writes only the two cohorts and their distributions, and
/// every new distribution is built canonical and totals its cohort's new
/// headcount, so a valid state stays valid without a whole-state check; the
/// settlement validates the whole root once, before it writes it.
fn apply_cohort_transfer(
    state: &mut SocietyState,
    intent: &CohortTransferIntent,
) -> Result<(), CanwuError> {
    let source_count = state
        .cohorts
        .get(&intent.source_cohort_id)
        .ok_or_else(|| invalid("unknown source cohort"))?
        .headcount;
    let destination_count = state
        .cohorts
        .get(&intent.destination_cohort_id)
        .ok_or_else(|| invalid("unknown destination cohort"))?
        .headcount;
    if intent.quantity >= source_count {
        return Err(invalid(
            "a cohort transfer would leave an invalid source cohort",
        ));
    }
    let destination_total = destination_count
        .checked_add(intent.quantity)
        .ok_or_else(|| invalid("cohort transfer would overflow the destination cohort"))?;
    let source_targets = cohort_distributions(state, &intent.source_cohort_id)
        .map(|distribution| distribution.target_id.clone())
        .collect::<BTreeSet<_>>();
    let destination_targets = cohort_distributions(state, &intent.destination_cohort_id)
        .map(|distribution| distribution.target_id.clone())
        .collect::<BTreeSet<_>>();
    if source_targets != destination_targets || source_targets.is_empty() {
        return Err(invalid(
            "source and destination target sets must be equal and non-empty",
        ));
    }
    let mut updates = Vec::with_capacity(2 * source_targets.len());
    for target_id in source_targets {
        let source_id = crate::distribution_id(&intent.source_cohort_id, &target_id);
        let destination_id = crate::distribution_id(&intent.destination_cohort_id, &target_id);
        let source = state
            .distributions
            .get(&source_id)
            .ok_or_else(|| invalid("missing source distribution"))?;
        let destination = state
            .distributions
            .get(&destination_id)
            .ok_or_else(|| invalid("missing destination distribution"))?;
        let allocations = proportional_allocations(&source.buckets, intent.quantity, source_count)?;
        let mut remaining = BTreeMap::new();
        let mut arrived = BTreeMap::new();
        for bucket in &destination.buckets {
            add_headcount(&mut arrived, bucket.profile, bucket.headcount)?;
        }
        for (bucket, moved) in source.buckets.iter().zip(allocations) {
            let left = bucket
                .headcount
                .checked_sub(moved)
                .ok_or_else(|| invalid("cohort transfer allocation exceeds its bucket"))?;
            add_headcount(&mut remaining, bucket.profile, left)?;
            add_headcount(&mut arrived, bucket.profile, moved)?;
        }
        updates.push((source_id, canonical_buckets(remaining)));
        updates.push((destination_id, canonical_buckets(arrived)));
    }
    for (id, buckets) in updates {
        state
            .distributions
            .get_mut(&id)
            .ok_or_else(|| invalid("a transferred distribution disappeared"))?
            .buckets = buckets;
    }
    state
        .cohorts
        .get_mut(&intent.source_cohort_id)
        .ok_or_else(|| invalid("the source cohort disappeared"))?
        .headcount = source_count - intent.quantity;
    state
        .cohorts
        .get_mut(&intent.destination_cohort_id)
        .ok_or_else(|| invalid("the destination cohort disappeared"))?
        .headcount = destination_total;
    state.invalidate_derived_state();
    Ok(())
}

/// The distributions of one cohort, from the contiguous key range its ID
/// starts.
pub(crate) fn cohort_distributions<'a>(
    state: &'a SocietyState,
    cohort_id: &'a str,
) -> impl Iterator<Item = &'a DispositionDistribution> + 'a {
    let prefix = format!("{cohort_id}::");
    state
        .distributions
        .range(prefix.clone()..)
        .take_while(move |(key, _)| key.starts_with(&prefix))
        .map(|(_, distribution)| distribution)
        .filter(move |distribution| distribution.cohort_id == cohort_id)
}

fn add_headcount(
    buckets: &mut BTreeMap<DispositionProfile, u64>,
    profile: DispositionProfile,
    headcount: u64,
) -> Result<(), CanwuError> {
    let entry = buckets.entry(profile).or_default();
    *entry = entry
        .checked_add(headcount)
        .ok_or_else(|| invalid("disposition bucket headcount overflowed"))?;
    Ok(())
}

/// Buckets in canonical profile order, without empty ones.
pub(crate) fn canonical_buckets(
    buckets: BTreeMap<DispositionProfile, u64>,
) -> Vec<DispositionBucket> {
    buckets
        .into_iter()
        .filter(|(_, headcount)| *headcount > 0)
        .map(|(profile, headcount)| DispositionBucket { profile, headcount })
        .collect()
}

fn proportional_allocations(
    buckets: &[DispositionBucket],
    quantity: u64,
    total: u64,
) -> Result<Vec<u64>, CanwuError> {
    let mut allocations = Vec::with_capacity(buckets.len());
    let mut remainders = Vec::new();
    let mut assigned = 0_u64;
    for (index, bucket) in buckets.iter().enumerate() {
        let product = bucket
            .headcount
            .checked_mul(quantity)
            .ok_or_else(|| invalid("cohort transfer allocation overflow"))?;
        let base = product / total;
        assigned += base;
        allocations.push(base);
        remainders.push((product % total, index));
    }
    remainders.sort_by(|a, b| b.cmp(a));
    let leftover = quantity
        .checked_sub(assigned)
        .and_then(|leftover| usize::try_from(leftover).ok())
        .ok_or_else(|| invalid("cohort transfer remainder is out of range"))?;
    for (_, index) in remainders.into_iter().take(leftover) {
        allocations[index] += 1;
    }
    Ok(allocations)
}

fn load_ledger(
    view: &SimulationView<'_>,
) -> Result<(SocietyCohortExchangeLedger, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&society_cohort_exchange_ledger_reference())?
    else {
        return Ok((SocietyCohortExchangeLedger::empty(), None));
    };
    let ledger = record.decode_payload::<SocietyCohortExchangeLedgerRecord>()?;
    ledger.validate()?;
    Ok((ledger, Some(record.clone())))
}

/// Admits one institutional policy decision as the alignment's pending
/// decision.
///
/// Every expected refusal uses an error code the engine records as a
/// rejected command attempt, so a refused decision never fails its boundary
/// or blocks the commands queued behind it: `InvalidPayload` for a malformed
/// decision, `InvalidAuthority` for a missing or mismatched authority (an
/// unknown alignment, or a run without a society root, grants none), and
/// `IdempotencyConflict` for a version that is not newer than the applied or
/// the pending one, which includes a resend. An invalid society record or
/// stored decision is not a refusal and still fails the boundary.
fn set_institutional_policy(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let decision: PolicyDecision = serde_json::from_value(payload.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("institutional policy payload could not be decoded: {error}"),
        )
    })?;
    validate_policy_decision(&decision)
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.message))?;
    let state = load_state(view)?.map(|(_, state)| state);
    let alignment = state
        .as_ref()
        .and_then(|state| state.institutional_alignments.get(&decision.alignment_id))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "unknown institutional policy authority alignment {}",
                    decision.alignment_id
                ),
            )
        })?;
    validate_policy_authority(context, alignment)?;
    if decision.decision_version <= alignment.last_decision_version {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            format!(
                "institutional decision version {} is not newer than applied version {}",
                decision.decision_version, alignment.last_decision_version
            ),
        ));
    }
    if let Some(pending) = view.component(
        &policy_decision_key(),
        &alignment.institution,
        &alignment.id,
    )? {
        let pending: PolicyDecision = serde_json::from_value(pending.clone()).map_err(|error| {
            invalid(format!(
                "stored institutional decision is malformed: {error}"
            ))
        })?;
        if decision.decision_version <= pending.decision_version {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "institutional decision version {} is not newer than pending version {}",
                    decision.decision_version, pending.decision_version
                ),
            ));
        }
    }

    Ok(vec![SystemDirective::SetComponent {
        state: policy_decision_key(),
        entity: alignment.institution.clone(),
        component: alignment.id.clone(),
        value: serde_json::to_value(&decision).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("institutional decision could not be encoded: {error}"),
            )
        })?,
        summary: format!(
            "Institutional alignment {} received a new policy",
            alignment.id
        ),
    }])
}

fn validate_policy_authority(
    context: &canwu_api::CommandContext,
    alignment: &InstitutionalAlignment,
) -> Result<(), CanwuError> {
    let Some(controller_id) = context.decision_controller_id.as_deref() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "institutional policy changes require a validated DecisionTicket controller",
        ));
    };
    if !matches!(
        &context.issuer,
        Issuer::Ai(issuer) | Issuer::Human(issuer) if issuer == controller_id
    ) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "institutional policy issuer does not match its validated DecisionTicket controller",
        ));
    }
    if context.authority.command_subject.as_ref() != Some(&alignment.institution) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "command subject {:?} cannot change institutional alignment {} owned by {:?}",
                context.authority.command_subject, alignment.id, alignment.institution
            ),
        ));
    }
    match (
        &context.authority.decision_origin,
        alignment.authorized_actor,
    ) {
        (DecisionOrigin::Actor { actor }, Some(authorized)) if *actor == authorized => {}
        (
            DecisionOrigin::Institution {
                institution,
                responsible_actor: Some(actor),
            },
            Some(authorized),
        ) if institution == &alignment.institution && *actor == authorized => {}
        (
            DecisionOrigin::Institution {
                institution,
                responsible_actor: _,
            },
            None,
        ) if institution == &alignment.institution => {}
        _ => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "decision origin {:?} cannot change institutional alignment {}",
                    context.authority.decision_origin, alignment.id
                ),
            ));
        }
    }
    Ok(())
}

/// Admits one pressure revision decision and queues it for the next society
/// settlement.
///
/// The decision is authorized exactly as `set_institutional_policy` is: a
/// validated `DecisionTicket` controller, the alignment's institution as
/// command subject, and a decision origin that matches the alignment's
/// authorized actor. Every named row must exist and lie within the
/// alignment's authority now, and is checked again when the decision settles.
/// A resend of the alignment's applied decision is accepted and queues
/// nothing; the same version with other content is an `IdempotencyConflict`.
/// The decision's place in the alignment's version sequence is settled by
/// the owner in admission order, because an earlier decision may still be
/// queued. While [`MAX_SOCIETY_INGRESS_QUEUE`] packets wait, a decision that
/// would queue one is refused as `ValueOutOfRange`, with a message that ends
/// with [`SOCIETY_INGRESS_QUEUE_FULL_REJECTION`].
///
/// Every refusal uses an error code the engine records as a rejected command
/// attempt (`InvalidPayload`, `InvalidAuthority`, `EntityNotFound`,
/// `IdempotencyConflict`, or `ValueOutOfRange`), so a refused decision never
/// fails its boundary.
fn revise_society_pressure(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let decision: PressureRevisionDecision =
        serde_json::from_value(payload.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("pressure revision payload could not be decoded: {error}"),
            )
        })?;
    decision
        .validate()
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.message))?;
    // Without a society root there is no alignment and so no authority.
    let state = load_state(view)?.map(|(_, state)| state);
    let alignment = state
        .as_ref()
        .and_then(|state| state.institutional_alignments.get(&decision.alignment_id))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "unknown pressure revision authority alignment {}",
                    decision.alignment_id
                ),
            )
        })?;
    validate_policy_authority(context, alignment)?;
    state
        .as_ref()
        .map_or(Ok(()), |state| {
            state.pressure_revision_scope(&decision, alignment)
        })
        .map_err(RevisionScopeError::into_error)?;

    let (ledger, _) = load_ledger(view)?;
    if let Some(applied) = ledger
        .pressure_revisions
        .get(&alignment.id)
        .and_then(|outcomes| outcomes.applied.as_ref())
        && applied.decision_version == decision.decision_version
    {
        if applied.digest == decision.digest()? {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            format!(
                "pressure revision version {} of alignment {} is already applied with other \
                 content",
                decision.decision_version, alignment.id
            ),
        ));
    }
    // Admission is the earliest point at which the issuer can learn that the
    // queue is full, so the command is refused here, as a recorded command
    // rejection. Its packet can still find the queue full when it is
    // admitted, if other packets fill it first; the intake then records a
    // `queue_full` outcome for the alignment.
    if usize::try_from(load_queue_head(view)?.0.queued)
        .map_or(true, |queued| queued >= MAX_SOCIETY_INGRESS_QUEUE)
    {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            format!(
                "{MAX_SOCIETY_INGRESS_QUEUE} society packets wait for settlement: \
                 {SOCIETY_INGRESS_QUEUE_FULL_REJECTION}"
            ),
        ));
    }
    let issuer = alignment.institution.clone();
    let packet = SocietyPressureRevisionV1 {
        decision,
        issuer: issuer.clone(),
        command: context.command_id,
    };
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: SOCIETY_PRESSURE_REVISION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(&packet).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("pressure revision could not be encoded: {error}"),
            )
        })?,
        affected: vec![issuer],
    }])
}

fn settle_society_one_by_one_keep(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    settle_society(
        view,
        context,
        TransferSettlement::OneByOne,
        OutcomeRetention::Keep,
    )
}

fn settle_society_one_by_one_latest(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    settle_society(
        view,
        context,
        TransferSettlement::OneByOne,
        OutcomeRetention::LatestSettlement,
    )
}

fn settle_society_batch_keep(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    settle_society(
        view,
        context,
        TransferSettlement::Batch,
        OutcomeRetention::Keep,
    )
}

fn settle_society_batch_latest(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    settle_society(
        view,
        context,
        TransferSettlement::Batch,
        OutcomeRetention::LatestSettlement,
    )
}

/// Settles every cohort transfer due at `at`, as `mode` prescribes, and
/// returns whether any settled.
///
/// A transfer whose cohorts changed after it was admitted (through a rebase
/// or a lifecycle delta, or, one by one, through an earlier transfer of this
/// settlement) settles as `stale_source`, and one that no longer fits settles
/// as a terminal rejection, instead of failing every later settlement. One by
/// one, the cohorts are compared with the in-progress state, in operation-ID
/// order, so of the due transfers that share a cohort one completes and the
/// others must be reissued. In a batch they are compared with the cohorts at
/// the start of the settlement, and the transfers apply in admission order
/// (the order of the commands that admitted them), each to the state the
/// earlier ones left, after a check that its quantity still fits its source
/// cohort.
fn settle_due_transfers(
    state: &mut SocietyState,
    ledger: &mut SocietyCohortExchangeLedger,
    pending: &SocietyCohortTransferPending,
    at: SimTime,
    mode: TransferSettlement,
) -> bool {
    let mut due: Vec<&PendingCohortTransfer> = pending
        .transfers
        .values()
        .filter(|transfer| transfer.intent.due_time <= at)
        .collect();
    // A transfer is checked against its two cohorts only, so a batch keeps
    // the cohorts as the settlement found them, not the whole state.
    let start = match mode {
        TransferSettlement::OneByOne => None,
        TransferSettlement::Batch => {
            due.sort_by(|left, right| {
                (left.command_id, &left.intent.operation_id)
                    .cmp(&(right.command_id, &right.intent.operation_id))
            });
            Some(state.cohorts.clone())
        }
    };
    let mut changed = false;
    for transfer in due {
        let operation_id = &transfer.intent.operation_id;
        // A transfer settled earlier stays in the pending record until the
        // next intake prunes it; its ledger outcome, not its (now changed)
        // source digest, settles it.
        if ledger.outcomes.contains_key(operation_id) {
            continue;
        }
        let cohorts = start.as_ref().unwrap_or(&state.cohorts);
        let digest = cohorts
            .get(&transfer.intent.source_cohort_id)
            .zip(cohorts.get(&transfer.intent.destination_cohort_id))
            .map(|(source, destination)| cohort_pair_digest(source, destination));
        let result = match digest {
            Some(Ok(digest)) if digest == transfer.source_digest => {
                let fits = state
                    .cohorts
                    .get(&transfer.intent.source_cohort_id)
                    .is_some_and(|source| transfer.intent.quantity < source.headcount);
                // A transfer that does not fit, or whose result would not be
                // a valid society state, is a terminal rejection; the apply
                // changes nothing then.
                if mode == TransferSettlement::Batch && !fits {
                    COHORT_TRANSFER_INSUFFICIENT_SOURCE
                } else if apply_cohort_transfer(state, &transfer.intent).is_ok() {
                    TRANSFER_COMPLETED
                } else {
                    TRANSFER_REJECTED
                }
            }
            Some(Ok(_)) => TRANSFER_STALE_SOURCE,
            // A cohort the pending transfer names no longer exists.
            Some(Err(_)) | None => TRANSFER_REJECTED,
        };
        ledger.outcomes.insert(
            operation_id.clone(),
            CohortTransferOutcome {
                operation_id: operation_id.clone(),
                source_record_version: transfer.intent.expected_source_version,
                source_cohort_id: transfer.intent.source_cohort_id.clone(),
                destination_cohort_id: transfer.intent.destination_cohort_id.clone(),
                quantity: transfer.intent.quantity,
                actor: transfer.actor,
                authority_alignment_id: transfer.intent.authority_alignment_id.clone(),
                due_time: transfer.intent.due_time,
                completed_at: at,
                result: result.to_owned(),
            },
        );
        changed = true;
    }
    changed
}

/// The society settlement: the one system that runs on the settlement
/// cadence and the only writer of the society root.
///
/// It settles due transfers, applies the queued rebases, lifecycle deltas,
/// and pressure revisions in admission order, applies institutional
/// decisions and transitions, and then completes every derived view those
/// changes cleared (mobilization candidates, aggregates, and the projections
/// of the observers whose estimates changed), so a settlement writes the
/// root at most once. A settlement that changes nothing writes nothing.
///
/// Each step keeps a valid state valid and checks only what it changes; the
/// whole root is validated once, before it is written.
fn settle_society(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    mode: TransferSettlement,
    retention: OutcomeRetention,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let before = state.clone();
    let (mut ledger, ledger_record) = load_ledger(view)?;
    let (pending, _) = load_pending(view)?;
    let transfer_changed =
        settle_due_transfers(&mut state, &mut ledger, &pending, context.at, mode);
    // Queued rebases, lifecycle deltas, and pressure revisions apply in
    // admission order, after transfers and before institutional decisions
    // and transitions.
    let queue = load_queue(view)?;
    let queue_consumed = !queue.entries.is_empty();
    let mut ledger_changed = transfer_changed;
    let mut events = Vec::new();
    for entry in &queue.entries {
        ledger_changed |= settle_queued_ingress(
            view,
            &mut state,
            &mut ledger,
            entry,
            context.at,
            &mut events,
        )?;
    }
    if retention == OutcomeRetention::LatestSettlement
        && queue.entries.iter().any(|entry| {
            matches!(
                entry.packet_type.as_str(),
                COHORT_REBASE_INGRESS | SOCIETY_LIFECYCLE_DELTA_INGRESS
            )
        })
    {
        ledger
            .rebases
            .retain(|_, outcome| outcome.settled_at >= context.at);
        ledger
            .lifecycle_deltas
            .retain(|_, outcome| outcome.settled_at >= context.at);
    }
    apply_pending_policies(view, &mut state)?;
    settle_valid_transitions(&mut state, context.at)?;
    complete_views(&before, &mut state, context.at);
    // A settlement that changes nothing leaves the state, and so the root
    // record, untouched.
    if state == before && !ledger_changed && !queue_consumed {
        return Ok(BoundaryProposal::default());
    }
    let mut proposal = if state == before {
        BoundaryProposal::default()
    } else {
        update_state(&record, state, "Settled society")?
    };
    if queue_consumed {
        proposal.directives.extend(queue.consumed()?);
    }
    proposal.directives.extend(events);
    if ledger_changed {
        ledger.validate()?;
        proposal.directives.push(record_write(
            society_cohort_exchange_ledger_reference(),
            &ledger,
            ledger_record.as_ref(),
            "Recorded society cohort exchange outcomes",
        )?);
    }
    Ok(proposal)
}

/// Applies one queued packet to the in-progress state, records its terminal
/// outcome, and returns whether the ledger changed. A rejected packet leaves
/// the state unchanged.
fn settle_queued_ingress(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
    ledger: &mut SocietyCohortExchangeLedger,
    entry: &QueuedSocietyIngress,
    at: SimTime,
    events: &mut Vec<BoundaryDirective>,
) -> Result<bool, CanwuError> {
    let key = entry.ingress.get().to_string();
    match entry.packet_type.as_str() {
        SOCIETY_PRESSURE_REVISION_INGRESS => {
            return settle_pressure_revision(Some(state), ledger, entry, at, events, None);
        }
        COHORT_REBASE_INGRESS => {
            if ledger.rebases.contains_key(&key) {
                return Ok(false);
            }
            let rebase =
                serde_json::from_value::<CohortHeadcountRebaseV1>(entry.payload.clone()).ok();
            let result = match (&entry.admission_rejection, &rebase) {
                (Some(reason), _) => Err(reason.clone()),
                (None, None) => Err(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned()),
                (None, Some(rebase)) => state
                    .rebase_cohort_in_place(&rebase.cohort_id, rebase.new_headcount)
                    .map_err(|error| error.message),
            };
            let (status, rejection, previous_headcount) = match result {
                Ok(previous) => (SocietyIngressStatus::Applied, None, Some(previous)),
                Err(reason) => (SocietyIngressStatus::Rejected, Some(reason), None),
            };
            ledger.rebases.insert(
                key,
                CohortHeadcountRebaseOutcome {
                    ingress: entry.ingress,
                    admitted_at: entry.admitted_at,
                    settled_at: at,
                    status,
                    rejection,
                    rebase,
                    previous_headcount,
                },
            );
        }
        SOCIETY_LIFECYCLE_DELTA_INGRESS => {
            if ledger.lifecycle_deltas.contains_key(&key) {
                return Ok(false);
            }
            let (status, rejection, blocked_releases) = match &entry.admission_rejection {
                Some(reason) => (
                    SocietyIngressStatus::Rejected,
                    Some(reason.clone()),
                    BTreeSet::new(),
                ),
                None => {
                    match serde_json::from_value::<SocietyLifecycleDeltaV1>(entry.payload.clone()) {
                        Err(_) => (
                            SocietyIngressStatus::Rejected,
                            Some(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned()),
                            BTreeSet::new(),
                        ),
                        Ok(delta) => apply_queued_lifecycle_delta(view, state, delta)?,
                    }
                }
            };
            ledger.lifecycle_deltas.insert(
                key,
                SocietyLifecycleDeltaOutcome {
                    ingress: entry.ingress,
                    admitted_at: entry.admitted_at,
                    settled_at: at,
                    status,
                    rejection,
                    blocked_releases,
                },
            );
        }
        other => {
            return Err(invalid(format!(
                "society ingress queue contains unsupported packet {other}"
            )));
        }
    }
    Ok(true)
}

/// Settles one queued pressure revision and returns whether the ledger
/// changed.
///
/// The decision applies when its alignment is still bound to the deciding
/// institution, its version is newer than the alignment's applied revision
/// version, its expected version is that applied version, and every row it
/// names still lies within the alignment's authority. A resend of the
/// applied decision changes nothing and records nothing. Every other packet
/// is rejected without changing society state. An applied or rejected
/// decision replaces the alignment's ledger outcome of that kind and is
/// emitted as an event naming the decision.
///
/// With `refused`, the intake records a decision it could not queue: it is
/// rejected with that reason, without reading `state`, under the same rules
/// for a resend and for a packet the revision command did not produce.
fn settle_pressure_revision(
    state: Option<&mut SocietyState>,
    ledger: &mut SocietyCohortExchangeLedger,
    entry: &QueuedSocietyIngress,
    at: SimTime,
    events: &mut Vec<BoundaryDirective>,
    refused: Option<&'static str>,
) -> Result<bool, CanwuError> {
    let packet = match &entry.admission_rejection {
        Some(_) => None,
        None => serde_json::from_value::<SocietyPressureRevisionV1>(entry.payload.clone()).ok(),
    };
    let Some(packet) = packet else {
        // The packet did not come from the revision command, so it names no
        // decision an alignment's ledger entry could record.
        let reason = entry
            .admission_rejection
            .as_deref()
            .unwrap_or(SOCIETY_INGRESS_MALFORMED_REJECTION);
        events.push(BoundaryDirective::Emit {
            event_type: SOCIETY_PRESSURE_REVISION_REJECTED_EVENT.to_owned(),
            summary: format!(
                "Rejected pressure revision ingress {}: {reason}",
                entry.ingress.get()
            ),
            affected: Vec::new(),
        });
        return Ok(false);
    };
    let decision = &packet.decision;
    let digest = decision.digest()?;
    let applied = ledger
        .pressure_revisions
        .get(&decision.alignment_id)
        .and_then(|outcomes| outcomes.applied.as_ref());
    if applied.is_some_and(|outcome| {
        outcome.decision_version == decision.decision_version && outcome.digest == digest
    }) {
        return Ok(false);
    }
    let applied_version = applied.map_or(0, |outcome| outcome.decision_version);
    let rejection = match (refused, state) {
        (Some(reason), _) => Some(reason),
        (None, None) => {
            return Err(invalid(
                "a queued pressure revision settles against the society state",
            ));
        }
        (None, Some(state)) => {
            let authority_holds = state
                .institutional_alignments
                .get(&decision.alignment_id)
                .is_some_and(|alignment| alignment.institution == packet.issuer);
            if !authority_holds {
                Some(PRESSURE_REVISION_AUTHORITY_REJECTION)
            } else if decision.decision_version <= applied_version {
                Some(PRESSURE_REVISION_SUPERSEDED_REJECTION)
            } else if decision.expected_version != applied_version {
                Some(PRESSURE_REVISION_STALE_REJECTION)
            } else if state.apply_pressure_revision(decision).is_err() {
                Some(PRESSURE_REVISION_SCOPE_REJECTION)
            } else {
                None
            }
        }
    };
    let outcome = PressureRevisionOutcome {
        decision_version: decision.decision_version,
        digest,
        issuer: packet.issuer.clone(),
        command: packet.command,
        ingress: entry.ingress,
        settled_at: at,
        rejection: rejection.map(str::to_owned),
    };
    let outcomes = ledger
        .pressure_revisions
        .entry(decision.alignment_id.clone())
        .or_default();
    let identity = format!(
        "pressure revision {} of alignment {} (command {}, ingress {})",
        decision.decision_version,
        decision.alignment_id,
        packet.command.get(),
        entry.ingress.get()
    );
    let (event_type, summary) = match rejection {
        None => {
            outcomes.applied = Some(outcome);
            (
                SOCIETY_PRESSURE_REVISED_EVENT,
                format!("Applied {identity}"),
            )
        }
        Some(reason) => {
            outcomes.rejected = Some(outcome);
            (
                SOCIETY_PRESSURE_REVISION_REJECTED_EVENT,
                format!("Rejected {identity}: {reason}"),
            )
        }
    };
    events.push(BoundaryDirective::Emit {
        event_type: event_type.to_owned(),
        summary,
        affected: Vec::new(),
    });
    Ok(true)
}

/// Applies one delivered lifecycle delta. A release is blocked, while the
/// rest of the delta still applies, when a released alignment carries a
/// stored institutional decision (decision components are permanent, so
/// releasing the alignment would orphan them) or when a live society
/// dependency would make the release fail; blocked targets are recorded on
/// the outcome.
fn apply_queued_lifecycle_delta(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
    mut delta: SocietyLifecycleDeltaV1,
) -> Result<(SocietyIngressStatus, Option<String>, BTreeSet<String>), CanwuError> {
    let mut blocked = BTreeSet::new();
    for set in delta
        .deactivations
        .iter()
        .filter(|set| delta.releases.contains(&set.target_id))
    {
        for alignment in &set.alignments {
            if let Some(stored) = state.institutional_alignments.get(&alignment.id)
                && view
                    .component(&policy_decision_key(), &stored.institution, &stored.id)?
                    .is_some()
            {
                blocked.insert(set.target_id.clone());
            }
        }
    }
    delta.releases.retain(|target| !blocked.contains(target));
    let result = match state.apply_lifecycle_delta(&delta) {
        Err(_) if !delta.releases.is_empty() => {
            blocked.append(&mut delta.releases);
            state.apply_lifecycle_delta(&delta)
        }
        result => result,
    };
    Ok(match result {
        Ok(()) => (SocietyIngressStatus::Applied, None, blocked),
        Err(error) => (
            SocietyIngressStatus::Rejected,
            Some(error.message),
            BTreeSet::new(),
        ),
    })
}

/// Adds every rebase, lifecycle delta, and pressure revision a boundary
/// admits to the owner-side queue.
///
/// Runs in phase 12 of any boundary that admits ingress, so the rebase stock
/// check sees the state committed through phase 11 of the admitting boundary.
/// A pressure revision is admissible only when its ingress record names the
/// revision command it carries as its cause. The next phase-7 settlement
/// consumes the queue.
///
/// The packets one boundary admits go into one new batch record, in the next
/// free slot, and the queue's head counts them; nothing already queued is
/// written again, so a boundary's queue evidence follows what it admits. A
/// boundary that admits none of these packets reads nothing. A packet that
/// arrives while [`MAX_SOCIETY_INGRESS_QUEUE`] wait is refused here, where
/// its sender can first learn of it: its terminal `queue_full` outcome is
/// recorded in the cohort exchange ledger, where every other outcome of its
/// kind is, and one event reports it.
fn intake_society_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut admitted = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        if let IngressPayload::Plugin {
            plugin,
            packet_type,
            ..
        } = &ingress.payload
            && is_queued_packet(plugin, packet_type)
        {
            admitted.push(ingress);
        }
    }
    if admitted.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let (head, head_record) = load_queue_head(view)?;
    let mut batch = SocietyIngressBatch::empty();
    let mut refused = Vec::new();
    for ingress in admitted {
        let IngressPayload::Plugin {
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        let entry = QueuedSocietyIngress {
            ingress: ingress.id,
            admitted_at: context.boundary_id,
            packet_type: packet_type.clone(),
            payload: payload.clone(),
            admission_rejection: admission_rejection(view, ingress, packet_type, payload)?,
        };
        let waiting = usize::try_from(head.queued)
            .ok()
            .and_then(|queued| queued.checked_add(batch.entries.len()));
        if waiting.is_none_or(|waiting| waiting >= MAX_SOCIETY_INGRESS_QUEUE) {
            refused.push(entry);
        } else {
            batch.entries.push(entry);
        }
    }
    let mut directives = Vec::new();
    if !batch.entries.is_empty() {
        directives.extend(queue_batch(view, head, head_record.as_ref(), &batch)?);
    }
    if !refused.is_empty() {
        let (mut ledger, ledger_record) = load_ledger(view)?;
        let mut ledger_changed = false;
        for entry in &refused {
            ledger_changed |=
                refuse_for_full_queue(&mut ledger, entry, context.at, &mut directives)?;
        }
        if ledger_changed {
            ledger.validate()?;
            directives.push(record_write(
                society_cohort_exchange_ledger_reference(),
                &ledger,
                ledger_record.as_ref(),
                "Recorded society ingress refused for a full queue",
            )?);
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Writes the packets one boundary admitted into the queue's next free slot
/// and counts them in the head.
fn queue_batch(
    view: &SimulationView<'_>,
    mut head: SocietyIngressQueue,
    head_record: Option<&DomainRecord>,
    batch: &SocietyIngressBatch,
) -> Result<[BoundaryDirective; 2], CanwuError> {
    let slot = head.batches;
    let (free, slot_record) = load_batch(view, slot)?;
    if !free.entries.is_empty() {
        return Err(invalid(
            "the next society ingress batch slot still holds packets",
        ));
    }
    batch.validate()?;
    let added = u32::try_from(batch.entries.len())
        .map_err(|_| invalid("society ingress batch exceeds the queue's capacity"))?;
    head.queued = head
        .queued
        .checked_add(added)
        .ok_or_else(|| invalid("society ingress queue count overflowed"))?;
    head.batches = head
        .batches
        .checked_add(1)
        .ok_or_else(|| invalid("society ingress batch count overflowed"))?;
    head.validate()?;
    Ok([
        record_write(
            society_ingress_batch_reference(slot),
            batch,
            slot_record.as_ref(),
            "Queued the society ingress one boundary admitted",
        )?,
        record_write(
            society_ingress_queue_reference(),
            &head,
            head_record,
            "Counted queued society ingress",
        )?,
    ])
}

/// The rejection a queued packet carries from its admission: a rebase whose
/// stock is not current or is a society record, a packet that does not
/// decode, or a pressure revision the revision command did not produce.
fn admission_rejection(
    view: &SimulationView<'_>,
    ingress: &canwu_api::IngressRecord,
    packet_type: &str,
    payload: &Value,
) -> Result<Option<String>, CanwuError> {
    Ok(if packet_type == COHORT_REBASE_INGRESS {
        match serde_json::from_value::<CohortHeadcountRebaseV1>(payload.clone()) {
            Err(_) => Some(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned()),
            Ok(rebase) => {
                let current = rebase.external_stock.record.kind.namespace
                    != crate::model::SOCIETY_NAMESPACE
                    && view.domain_record_version_is_current(&rebase.external_stock)?;
                rebase_admission_rejection(&rebase, current)
            }
        }
    } else if packet_type == SOCIETY_PRESSURE_REVISION_INGRESS {
        pressure_revision_admission_rejection(payload, ingress.cause.as_ref())
    } else {
        None
    })
}

/// Records the terminal outcome of a packet refused because the queue was
/// full, where every other outcome of its kind is recorded, and reports it
/// with one event. Returns whether the ledger changed.
///
/// A rebase or lifecycle delta is rejected under its ingress ID. A pressure
/// revision becomes its alignment's latest rejected decision, as a decision
/// refused at settlement does, unless it is a resend of the applied decision,
/// which changes nothing, or was not produced by the revision command, which
/// names no decision an alignment could record.
fn refuse_for_full_queue(
    ledger: &mut SocietyCohortExchangeLedger,
    entry: &QueuedSocietyIngress,
    at: SimTime,
    events: &mut Vec<BoundaryDirective>,
) -> Result<bool, CanwuError> {
    let key = entry.ingress.get().to_string();
    let rejection = Some(SOCIETY_INGRESS_QUEUE_FULL_REJECTION.to_owned());
    match entry.packet_type.as_str() {
        SOCIETY_PRESSURE_REVISION_INGRESS => {
            return settle_pressure_revision(
                None,
                ledger,
                entry,
                at,
                events,
                Some(SOCIETY_INGRESS_QUEUE_FULL_REJECTION),
            );
        }
        COHORT_REBASE_INGRESS => {
            ledger.rebases.insert(
                key,
                CohortHeadcountRebaseOutcome {
                    ingress: entry.ingress,
                    admitted_at: entry.admitted_at,
                    settled_at: at,
                    status: SocietyIngressStatus::Rejected,
                    rejection,
                    rebase: serde_json::from_value(entry.payload.clone()).ok(),
                    previous_headcount: None,
                },
            );
        }
        SOCIETY_LIFECYCLE_DELTA_INGRESS => {
            ledger.lifecycle_deltas.insert(
                key,
                SocietyLifecycleDeltaOutcome {
                    ingress: entry.ingress,
                    admitted_at: entry.admitted_at,
                    settled_at: at,
                    status: SocietyIngressStatus::Rejected,
                    rejection,
                    blocked_releases: BTreeSet::new(),
                },
            );
        }
        other => {
            return Err(invalid(format!(
                "society intake refused unsupported packet {other}"
            )));
        }
    }
    events.push(BoundaryDirective::Emit {
        event_type: SOCIETY_INGRESS_REJECTED_EVENT.to_owned(),
        summary: format!(
            "Rejected society ingress {}: {SOCIETY_INGRESS_QUEUE_FULL_REJECTION}",
            entry.ingress.get()
        ),
        affected: Vec::new(),
    });
    Ok(true)
}

/// A write of one record of the plugin: an update of `existing`, or the
/// record's creation.
fn record_write<T: DomainRecordType>(
    reference: canwu_api::TypedDomainRecordRef<T>,
    payload: &T::Payload,
    existing: Option<&DomainRecord>,
    summary: &str,
) -> Result<BoundaryDirective, CanwuError>
where
    T::Payload: serde::Serialize,
{
    let record = DomainRecordDraft::from_typed(reference, payload)?;
    Ok(BoundaryDirective::MutateRecord {
        mutation: match existing {
            Some(existing) => DomainRecordMutation::Update {
                record,
                expected_version: existing.version,
            },
            None => DomainRecordMutation::Create { record },
        },
        summary: summary.to_owned(),
    })
}

/// The queue's head as the view reads it, and its record; an empty head when
/// no packet has ever been queued.
///
/// Only the intake adds packets, so a non-empty queue whose head the initial
/// scenario established was never admitted, and is refused.
fn load_queue_head(
    view: &SimulationView<'_>,
) -> Result<(SocietyIngressQueue, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&society_ingress_queue_reference())? else {
        return Ok((SocietyIngressQueue::empty(), None));
    };
    let head = record.decode_payload::<SocietyIngressQueueRecord>()?;
    head.validate()?;
    if head.queued > 0
        && view
            .current_domain_record_version(&record.reference)?
            .is_none_or(|version| {
                version.established_by == canwu_api::DomainRecordVersionSource::InitialScenario
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "an initial scenario cannot seed admitted society ingress",
        ));
    }
    Ok((head, Some(record.clone())))
}

/// The batch in queue slot `slot` and its record; an empty batch when the
/// slot has never been used.
fn load_batch(
    view: &SimulationView<'_>,
    slot: u32,
) -> Result<(SocietyIngressBatch, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&society_ingress_batch_reference(slot))? else {
        return Ok((SocietyIngressBatch::empty(), None));
    };
    let batch = record.decode_payload::<SocietyIngressBatchRecord>()?;
    batch.validate()?;
    Ok((batch, Some(record.clone())))
}

/// The owner-side queue as a settlement consumes it.
struct LoadedQueue {
    head: Option<DomainRecord>,
    /// The records of the batches in use, in slot order.
    batches: Vec<DomainRecord>,
    /// Every queued packet, in admission order.
    entries: Vec<QueuedSocietyIngress>,
}

impl LoadedQueue {
    /// The writes that empty every batch in use and reset the head, so the
    /// slots are reused from the first.
    fn consumed(&self) -> Result<Vec<BoundaryDirective>, CanwuError> {
        let mut directives = Vec::with_capacity(self.batches.len() + 1);
        for (slot, record) in (0..).zip(&self.batches) {
            directives.push(record_write(
                society_ingress_batch_reference(slot),
                &SocietyIngressBatch::empty(),
                Some(record),
                "Consumed one batch of queued society ingress",
            )?);
        }
        directives.push(record_write(
            society_ingress_queue_reference(),
            &SocietyIngressQueue::empty(),
            self.head.as_ref(),
            "Consumed queued society ingress",
        )?);
        Ok(directives)
    }
}

/// Reads the queue: its head and, only when packets wait, the batches in
/// use, checked against the head.
fn load_queue(view: &SimulationView<'_>) -> Result<LoadedQueue, CanwuError> {
    let (head, head_record) = load_queue_head(view)?;
    let mut batches = Vec::new();
    let mut records = Vec::new();
    for slot in 0..head.batches {
        let (batch, record) = load_batch(view, slot)?;
        records.push(record.ok_or_else(|| invalid("a society ingress batch in use is missing"))?);
        batches.push(batch);
    }
    Ok(LoadedQueue {
        head: head_record,
        batches: records,
        entries: queue_entries(&head, batches)?,
    })
}

/// The slot number a batch record ID names: four decimal digits.
fn batch_slot(id: &str) -> Result<u32, CanwuError> {
    id.parse::<u32>()
        .ok()
        .filter(|slot| {
            id.len() == 4
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && usize::try_from(*slot).is_ok_and(|slot| slot < MAX_SOCIETY_INGRESS_QUEUE)
        })
        .ok_or_else(|| invalid(format!("society ingress batch ID {id} is not a queue slot")))
}

/// Completes, after the settlement's changes, every derived view they
/// cleared, at the settlement's time: the mobilization candidates, the
/// aggregates, and the projections. A view that is still current keeps its
/// time.
///
/// An observer whose estimate the changes left as it was keeps its
/// projection and its `observed_at`; every other observer receives a
/// projection observed now. Until 0.18.0 the views were completed by three
/// later systems, each with its own write of the root; the result is the
/// same root.
fn complete_views(before: &SocietyState, state: &mut SocietyState, at: SimTime) {
    let estimates = carry_unchanged_projections(before, state, at);
    if state.last_mobilization_at.is_none() {
        state.mobilization_candidates = compute_mobilization_candidates(state, at);
        state.last_mobilization_at = Some(at);
    }
    if state.last_aggregation_at.is_none() {
        state.aggregates = compute_aggregates(state);
        state.last_aggregation_at = Some(at);
    }
    if state.last_projection_at.is_none() {
        // The estimates read the distributions, the organizations, and the
        // observer profiles, which nothing changes after the carry.
        let mut projections = estimates.unwrap_or_else(|| compute_projections(state, at));
        for (key, projection) in &mut projections {
            if let Some(kept) = state.projections.get(key)
                && kept.actor == projection.actor
                && kept.entries == projection.entries
            {
                projection.observed_at = kept.observed_at;
            }
        }
        state.projections = projections;
        state.last_projection_at = Some(at);
    }
}

/// Restores, after the settlement's changes, the projection of every
/// observer whose estimate those changes left as it was, leaves the
/// projections of the others cleared, and returns the estimates it computed
/// to compare them.
///
/// An observer's `observed_at` therefore moves only in a settlement that
/// changes that observer's own entries. Clearing every projection whenever
/// any distribution moved would stamp every observer in that settlement and
/// so tell an observer that something outside its scope, or beyond what it
/// can detect, had changed.
fn carry_unchanged_projections(
    before: &SocietyState,
    state: &mut SocietyState,
    at: SimTime,
) -> Option<BTreeMap<String, SocietyProjection>> {
    if before.projections.is_empty() {
        return None;
    }
    // An estimate reads the distributions, the organizations, and the
    // observer profiles, and nothing else.
    if state.distributions == before.distributions
        && state.organizations == before.organizations
        && state.observer_profiles == before.observer_profiles
    {
        state.projections.clone_from(&before.projections);
        state.last_projection_at = before.last_projection_at;
        return None;
    }
    let estimates = compute_projections(state, at);
    state.projections = before
        .projections
        .iter()
        .filter(|(key, kept)| {
            estimates.get(*key).is_some_and(|estimate| {
                estimate.actor == kept.actor && estimate.entries == kept.entries
            })
        })
        .map(|(key, kept)| (key.clone(), kept.clone()))
        .collect();
    state.last_projection_at = before
        .last_projection_at
        .filter(|_| state.projections.len() == estimates.len());
    Some(estimates)
}

/// Applies every intergroup relation revision a boundary admits, in
/// admission order, in phase 7 of that boundary, so the systems of later
/// phases read the revised relations.
///
/// The system is total over what a packet can carry: a revision that does
/// not decode, is malformed, or that the relations refuse (a stale revision,
/// a changed identity, an unknown kind, class, or region, or a full set)
/// changes nothing and is reported by one
/// [`INTERGROUP_RELATION_REJECTED_EVENT`]; the boundary commits. A boundary
/// whose revisions all apply writes the relations record once. It never
/// reads or writes the society root's population state: it reads only the
/// cohorts' classifications, which never change at runtime. Only an invalid
/// relations record or society root fails the boundary.
fn intake_intergroup_relations(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut packets = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } = &ingress.payload
            && plugin == PLUGIN_NAME
            && packet_type == INTERGROUP_RELATION_INGRESS
        {
            packets.push((ingress.id, payload));
        }
    }
    if packets.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let (mut relations, record) = load_relations(view)?;
    let classes = match view.typed_domain_record(&society_state_reference())? {
        Some(root) => cohort_classes(
            root_map::<SocietyCohort>(&root.payload, "cohorts")?
                .values()
                .collect::<Vec<_>>(),
        ),
        None => BTreeSet::new(),
    };
    let mut directives = Vec::new();
    let mut changed = false;
    for (ingress, payload) in packets {
        let revision = serde_json::from_value::<IntergroupRelationRevisionV1>(payload.clone())
            .ok()
            .filter(|revision| revision.validate().is_ok());
        let rejection = match revision {
            None => Some((SOCIETY_INGRESS_MALFORMED_REJECTION, None)),
            Some(revision) => {
                let live_regions = live_regions(view, revision.regions())?;
                relations
                    .apply_revision(&revision, &classes, &live_regions, ingress, context.at)
                    .err()
                    .map(|rejection| (rejection.reason, rejection.relation))
            }
        };
        match rejection {
            None => changed = true,
            Some((reason, relation)) => directives.push(BoundaryDirective::Emit {
                event_type: INTERGROUP_RELATION_REJECTED_EVENT.to_owned(),
                summary: match relation {
                    Some(relation) => format!(
                        "Rejected intergroup relation ingress {} at relation {relation}: {reason}",
                        ingress.get()
                    ),
                    None => format!(
                        "Rejected intergroup relation ingress {}: {reason}",
                        ingress.get()
                    ),
                },
                affected: Vec::new(),
            }),
        }
    }
    if changed {
        relations.validate()?;
        let draft = relations.record_draft()?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: match &record {
                Some(existing) => DomainRecordMutation::Update {
                    record: draft,
                    expected_version: existing.version,
                },
                None => DomainRecordMutation::Create { record: draft },
            },
            summary: "Revised intergroup relations".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The regions among `regions` that a new edge may name now: a territory or
/// a government that exists, or an active domain entity record.
fn live_regions(
    view: &SimulationView<'_>,
    regions: BTreeSet<EntityRef>,
) -> Result<BTreeSet<EntityRef>, CanwuError> {
    let mut live = BTreeSet::new();
    for region in regions {
        let exists = match &region {
            EntityRef::Territory(id) => view.territory(*id)?.is_some(),
            EntityRef::Government(id) => view.government(*id)?.is_some(),
            EntityRef::Domain(reference) => view.domain_record(reference)?.is_some_and(|record| {
                record.class == DomainRecordClass::Entity && record.is_active()
            }),
            _ => false,
        };
        if exists && is_region_kind(&region) {
            live.insert(region);
        }
    }
    Ok(live)
}

/// One map of the committed society root, decoded alone.
fn root_map<T: serde::de::DeserializeOwned>(
    root: &Value,
    map: &str,
) -> Result<BTreeMap<String, T>, CanwuError> {
    root.get(map).map_or_else(
        || Ok(BTreeMap::new()),
        |entries| {
            BTreeMap::<String, T>::deserialize(entries)
                .map_err(|error| invalid(format!("society root {map} are malformed: {error}")))
        },
    )
}

/// The society's intergroup relations as the view reads them, and their
/// record; no relations when the record does not exist yet.
fn load_relations(
    view: &SimulationView<'_>,
) -> Result<(IntergroupRelations, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&intergroup_relations_reference())? else {
        return Ok((IntergroupRelations::empty(), None));
    };
    Ok((decode_relations(record)?, Some(record.clone())))
}

/// Decodes and validates a relations record, which binds no references.
pub(crate) fn decode_relations(record: &DomainRecord) -> Result<IntergroupRelations, CanwuError> {
    let relations = record.decode_payload::<IntergroupRelationsRecord>()?;
    relations.validate()?;
    IntergroupRelations::validate_record_binding(record)?;
    Ok(relations)
}

fn intergroup_relations_key() -> StateKey {
    StateKey::new(
        IntergroupRelationsRecord::NAMESPACE,
        IntergroupRelationsRecord::NAME,
    )
}

fn apply_pending_policies(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    for alignment in state.institutional_alignments.values_mut() {
        let Some(value) = view.component(
            &policy_decision_key(),
            &alignment.institution,
            &alignment.id,
        )?
        else {
            continue;
        };
        let decision: PolicyDecision = serde_json::from_value(value.clone()).map_err(|error| {
            invalid(format!(
                "stored institutional decision is malformed: {error}"
            ))
        })?;
        validate_policy_decision(&decision)?;
        if decision.alignment_id != alignment.id {
            return Err(invalid(format!(
                "institutional decision {} was stored on alignment {}",
                decision.alignment_id, alignment.id
            )));
        }
        if decision.decision_version <= alignment.last_decision_version {
            continue;
        }
        alignment.support_per_mille = decision.support_per_mille;
        alignment.enforcement_per_mille = decision.enforcement_per_mille;
        alignment.access_grant_per_mille = decision.access_grant_per_mille;
        alignment.last_decision_version = decision.decision_version;
    }
    Ok(())
}

fn update_state(
    record: &DomainRecord,
    mut state: SocietyState,
    summary: &str,
) -> Result<BoundaryProposal, CanwuError> {
    state.canonicalize()?;
    state.validate()?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.record_draft()?,
                expected_version: record.version,
            },
            summary: summary.to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn load_state(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, SocietyState)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&society_state_reference())? else {
        return Ok(None);
    };
    let mut state = record.decode_payload::<SocietyStateRecord>()?;
    state.canonicalize()?;
    state.validate()?;
    state.validate_at(view.time())?;
    state.validate_record_binding(record)?;
    Ok(Some((record.clone(), state)))
}

pub(crate) fn validate_policy_decision(decision: &PolicyDecision) -> Result<(), CanwuError> {
    if decision.decision_version == 0
        || decision.support_per_mille > 1_000
        || decision.enforcement_per_mille > 1_000
        || decision.access_grant_per_mille > 1_000
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "institutional policy fields must be versioned and within permille bounds",
        ));
    }
    Ok(())
}

fn cohort_exchange_ledger_key() -> StateKey {
    StateKey::new(
        SocietyCohortExchangeLedgerRecord::NAMESPACE,
        SocietyCohortExchangeLedgerRecord::NAME,
    )
}

fn society_state_key() -> StateKey {
    StateKey::new(SocietyStateRecord::NAMESPACE, SocietyStateRecord::NAME)
}

pub(crate) fn policy_decision_key() -> StateKey {
    StateKey::new("canwu.society", "policy-decisions")
}

/// Component state holding versioned institutional policy decisions that the
/// society plugin applies at its next settlement. A reader (for example
/// a lifecycle provider) declares this key to observe accepted decisions.
#[must_use]
pub fn society_policy_decision_state_key() -> StateKey {
    policy_decision_key()
}

fn ingress_queue_key() -> StateKey {
    StateKey::new(
        SocietyIngressQueueRecord::NAMESPACE,
        SocietyIngressQueueRecord::NAME,
    )
}

fn ingress_batch_key() -> StateKey {
    StateKey::new(
        SocietyIngressBatchRecord::NAMESPACE,
        SocietyIngressBatchRecord::NAME,
    )
}

fn rebase_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            ("cohort_id".to_owned(), required(PayloadValueType::String)),
            (
                "new_headcount".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "external_stock".to_owned(),
                required(PayloadValueType::Object),
            ),
            ("reason".to_owned(), required(PayloadValueType::String)),
        ]),
        allow_additional: false,
    }
}

fn cohort_transfer_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "operation_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "authority_alignment_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "source_cohort_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "destination_cohort_id".to_owned(),
                required(PayloadValueType::String),
            ),
            ("quantity".to_owned(), required(PayloadValueType::Integer)),
            (
                "expected_source_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("due_time".to_owned(), required(PayloadValueType::Integer)),
        ]),
        allow_additional: false,
    }
}

fn exchange_ledger_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "schema_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("outcomes".to_owned(), required(PayloadValueType::Object)),
            ("rebases".to_owned(), optional(PayloadValueType::Object)),
            (
                "lifecycle_deltas".to_owned(),
                optional(PayloadValueType::Object),
            ),
            (
                "pressure_revisions".to_owned(),
                optional(PayloadValueType::Object),
            ),
        ]),
        allow_additional: false,
    }
}

fn pressure_revision_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "alignment_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "decision_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "expected_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("policies".to_owned(), optional(PayloadValueType::Array)),
            (
                "influence_edges".to_owned(),
                optional(PayloadValueType::Array),
            ),
            (
                "organizations".to_owned(),
                optional(PayloadValueType::Array),
            ),
        ]),
        allow_additional: false,
    }
}

fn policy_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "access_grant_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "alignment_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "decision_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "enforcement_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "support_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
        ]),
        allow_additional: false,
    }
}

fn society_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            ("aggregates".to_owned(), required(PayloadValueType::Object)),
            ("cohorts".to_owned(), required(PayloadValueType::Object)),
            (
                "distributions".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "influence_edges".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "institutional_alignments".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "last_aggregation_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_mobilization_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_projection_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_transition_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "mobilization_candidates".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "observer_profiles".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "organization_relations".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "organizations".to_owned(),
                required(PayloadValueType::Object),
            ),
            ("policies".to_owned(), required(PayloadValueType::Object)),
            ("projections".to_owned(), required(PayloadValueType::Object)),
            (
                "relation_vocabulary".to_owned(),
                optional(PayloadValueType::Array),
            ),
            ("remainders".to_owned(), required(PayloadValueType::Object)),
            (
                "schema_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("targets".to_owned(), required(PayloadValueType::Object)),
            (
                "topology_passes".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "transition_rules".to_owned(),
                required(PayloadValueType::Object),
            ),
        ]),
        allow_additional: false,
    }
}

const fn required(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: true,
    }
}

const fn optional(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{
        Canwu, CommandAuthority, CommandContext, CommandId, CommandIngress, CommandPolicyContext,
        EntityRef, Issuer, SimTime,
    };

    #[test]
    fn decision_policy_authority_binds_controller_origin_and_subject() {
        let ids = Canwu::demo_ids();
        let alignment = InstitutionalAlignment {
            id: "alignment".to_owned(),
            institution: EntityRef::Government(ids.government),
            target_id: "target".to_owned(),
            affected_cohorts: std::collections::BTreeSet::default(),
            support_per_mille: 0,
            enforcement_per_mille: 0,
            access_grant_per_mille: 0,
            authorized_actor: Some(ids.commander),
            last_decision_version: 0,
        };
        let context = |subject| CommandContext {
            issuer: Issuer::Ai("controller".to_owned()),
            authority: CommandAuthority {
                decision_origin: DecisionOrigin::Actor {
                    actor: ids.commander,
                },
                seat_id: None,
                permission_profile_id: None,
                command_subject: subject,
            },
            decision_controller_id: Some("controller".to_owned()),
            run_policy: CommandPolicyContext::LegacyUnspecified,
            ingress: CommandIngress::LiveRequest,
            attempt_id: None,
            command_id: CommandId::new(1),
            request_id: None,
            revision: 0,
            simulation_time: SimTime::EPOCH,
            expected_revision: None,
            expected_time: None,
        };

        assert!(validate_policy_authority(&context(None), &alignment).is_err());
        assert!(
            validate_policy_authority(&context(Some(EntityRef::Army(ids.army))), &alignment)
                .is_err()
        );
        assert!(
            validate_policy_authority(
                &context(Some(EntityRef::Government(ids.government))),
                &alignment,
            )
            .is_ok()
        );
    }
}
