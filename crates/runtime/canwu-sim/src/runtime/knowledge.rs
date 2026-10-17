use super::records::{DomainRecordClass, DomainRecordSchemas};
use super::{
    CanwuError, ErrorCode, PayloadSchema, PersistentDomainRecordStore, RuntimeCurrentState,
    SimulationSnapshot, canonical_text, component_key, is_canonical_hash,
};
use canwu_core::{
    CoreEntityKind, DomainRecordKind, EntityRef, KnowledgeHolderPolicy, KnowledgeHolderRef,
    KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId,
};
use canwu_knowledge::{
    DEFAULT_KNOWLEDGE_PAGE_SIZE, HolderLedgers, KnowledgeEncodingError, KnowledgeOrigin,
    KnowledgeRecordBody, KnowledgeRecordDraft, KnowledgeRetention, KnowledgeSubject,
    KnowledgeSubjectTarget, MAX_KNOWLEDGE_PAGE_SIZE, StagedKnowledgeEntries,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Engine limits on generic knowledge.
///
/// A boundary has one publication budget of record bodies, holder entries,
/// and payload bytes, spent in dispatch order by every publishing system: a
/// system reads what is left with
/// [`crate::SimulationView::knowledge_publication_budget`]. A plugin may hold
/// a share of it ([`KnowledgePublicationReservation`]), which other plugins'
/// systems cannot take. A proposal that exceeds what it was left, or a directive that
/// exceeds its per-directive bounds, fails the boundary with
/// [`ErrorCode::KnowledgeLimitExceeded`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeLimitsV1 {
    pub schemas_per_plugin: usize,
    /// Record bodies (drafts) in one `PublishKnowledge` directive, over all
    /// of its groups.
    pub records_per_batch: usize,
    /// Holder entries in one directive: the sum over its groups of holders
    /// times drafts.
    pub holder_entries_per_batch: usize,
    /// Record bodies one boundary may store.
    pub records_per_boundary: usize,
    /// Holder entries one boundary may add.
    pub holder_entries_per_boundary: usize,
    /// Bytes of payload the bodies one boundary stores may carry, each
    /// charged [`KnowledgeRecordDraft::payload_bytes`].
    pub payload_bytes_per_boundary: usize,
    pub payload_bytes_per_record: usize,
    pub relations_per_record: usize,
    pub text_bytes: usize,
    pub max_page_size: u32,
    pub queries_per_batch: usize,
    pub ids_per_direct_get: usize,
    pub default_page_size: u32,
    pub relation_trace_depth: u32,
    pub relation_graph_records: usize,
}

impl KnowledgeLimitsV1 {
    pub const CURRENT: Self = Self {
        schemas_per_plugin: 256,
        records_per_batch: 1_000,
        holder_entries_per_batch: 4_096,
        records_per_boundary: 32_768,
        holder_entries_per_boundary: 32_768,
        payload_bytes_per_boundary: 64 * 1_024 * 1_024,
        payload_bytes_per_record: 65_536,
        relations_per_record: 64,
        text_bytes: 1_024,
        max_page_size: MAX_KNOWLEDGE_PAGE_SIZE,
        queries_per_batch: 64,
        ids_per_direct_get: 1_000,
        default_page_size: DEFAULT_KNOWLEDGE_PAGE_SIZE,
        relation_trace_depth: 32,
        relation_graph_records: 10_000,
    };
}

/// What a boundary may still publish at a system's cut, with the
/// per-directive bounds a publisher splits its groups against. Returned by
/// [`crate::SimulationView::knowledge_publication_budget`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgePublicationBudget {
    /// Record bodies the boundary may still store.
    pub records: usize,
    /// Holder entries the boundary may still add.
    pub holder_entries: usize,
    /// Payload bytes of new bodies the boundary may still take.
    pub payload_bytes: usize,
    /// [`KnowledgeLimitsV1::records_per_batch`].
    pub records_per_batch: usize,
    /// [`KnowledgeLimitsV1::holder_entries_per_batch`].
    pub holder_entries_per_batch: usize,
}

impl KnowledgePublicationBudget {
    /// Whether one more directive of `records` bodies, `holder_entries`
    /// entries, and `payload_bytes` bytes of payload fits: within the
    /// per-directive bounds and within what is left.
    #[must_use]
    pub const fn admits(
        &self,
        records: usize,
        holder_entries: usize,
        payload_bytes: usize,
    ) -> bool {
        records <= self.records_per_batch
            && holder_entries <= self.holder_entries_per_batch
            && records <= self.records
            && holder_entries <= self.holder_entries
            && payload_bytes <= self.payload_bytes
    }

    /// Takes one directive's bodies, entries, and payload bytes from what is
    /// left when [`Self::admits`] them, and says whether it did.
    #[must_use]
    pub const fn take(
        &mut self,
        records: usize,
        holder_entries: usize,
        payload_bytes: usize,
    ) -> bool {
        if !self.admits(records, holder_entries, payload_bytes) {
            return false;
        }
        self.records -= records;
        self.holder_entries -= holder_entries;
        self.payload_bytes -= payload_bytes;
        true
    }
}

/// A share of every boundary's knowledge publication budget held for one
/// plugin's systems, declared at registration
/// ([`crate::PluginRegistrar::reserve_knowledge_publication`]; a host
/// reserves for a plugin it does not write by registering it through
/// [`crate::KnowledgeReservedPlugin`]).
///
/// The plugin must register a knowledge-writing boundary system (phase 4 or
/// 13 with a knowledge write grant). In every boundary where one of those
/// systems is due, whatever the others' cadences, the share is held for the
/// whole boundary: what the plugin has not yet spent of it is left out of
/// the budget every system of another plugin reads, whatever the dispatch
/// order, so a publisher that runs earlier and spends all it is shown cannot
/// take it. A plugin publishing in both phases has one share for both; when
/// only its phase-4 system is due, what it leaves unspent stays held through
/// phase 13. A boundary in which none of its knowledge-writing systems is
/// due holds nothing for it. The plugin's own systems read the whole of what
/// is left, their unspent share included. A publication counts against its
/// plugin's share first and, past it, against the unreserved budget. Without
/// reservations, the default, the budget is shared in dispatch order as
/// before.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePublicationReservation {
    /// Record bodies held for the plugin in each boundary.
    pub records: usize,
    /// Holder entries held for the plugin in each boundary.
    pub holder_entries: usize,
    /// Payload bytes of new bodies held for the plugin in each boundary,
    /// charged as [`KnowledgeRecordDraft::payload_bytes`] charges them.
    pub payload_bytes: usize,
}

impl KnowledgePublicationReservation {
    #[must_use]
    pub const fn new(records: usize, holder_entries: usize, payload_bytes: usize) -> Self {
        Self {
            records,
            holder_entries,
            payload_bytes,
        }
    }

    /// Whether it holds nothing; registration refuses such a reservation.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.records == 0 && self.holder_entries == 0 && self.payload_bytes == 0
    }
}

/// The reservations a boundary holds, by plugin name.
pub(crate) type KnowledgePublicationReservations =
    BTreeMap<String, KnowledgePublicationReservation>;

/// Whether `reservations` together fit the per-boundary limits.
pub(crate) fn knowledge_publication_reservations_fit<'a>(
    reservations: impl IntoIterator<Item = &'a KnowledgePublicationReservation>,
) -> bool {
    let limits = KnowledgeLimitsV1::CURRENT;
    let total =
        reservations
            .into_iter()
            .fold(KnowledgeBudgetUsage::default(), |total, reservation| {
                total.plus(KnowledgeBudgetUsage {
                    records: reservation.records,
                    entries: reservation.holder_entries,
                    payload_bytes: reservation.payload_bytes,
                })
            });
    total.records <= limits.records_per_boundary
        && total.entries <= limits.holder_entries_per_boundary
        && total.payload_bytes <= limits.payload_bytes_per_boundary
}

/// What the knowledge publications of a boundary, or of one proposal, take
/// from the budget.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct KnowledgeBudgetUsage {
    pub(crate) records: usize,
    pub(crate) entries: usize,
    pub(crate) payload_bytes: usize,
}

impl KnowledgeBudgetUsage {
    /// The budget left after this usage, with nothing reserved.
    pub(crate) const fn remaining(self) -> KnowledgePublicationBudget {
        self.remaining_less(Self {
            records: 0,
            entries: 0,
            payload_bytes: 0,
        })
    }

    /// The budget left after this usage and `withheld`.
    const fn remaining_less(self, withheld: Self) -> KnowledgePublicationBudget {
        let limits = KnowledgeLimitsV1::CURRENT;
        KnowledgePublicationBudget {
            records: limits
                .records_per_boundary
                .saturating_sub(self.records)
                .saturating_sub(withheld.records),
            holder_entries: limits
                .holder_entries_per_boundary
                .saturating_sub(self.entries)
                .saturating_sub(withheld.entries),
            payload_bytes: limits
                .payload_bytes_per_boundary
                .saturating_sub(self.payload_bytes)
                .saturating_sub(withheld.payload_bytes),
            records_per_batch: limits.records_per_batch,
            holder_entries_per_batch: limits.holder_entries_per_batch,
        }
    }

    const fn plus(self, other: Self) -> Self {
        Self {
            records: self.records.saturating_add(other.records),
            entries: self.entries.saturating_add(other.entries),
            payload_bytes: self.payload_bytes.saturating_add(other.payload_bytes),
        }
    }
}

/// What a boundary's knowledge publications have taken from its budget, in
/// total and by each plugin that holds a reservation, in dispatch order.
///
/// A system of plugin `p` may take what the limits leave after everything
/// taken so far and after the unspent shares of the other plugins'
/// reservations. Since the reservations together fit the limits and every
/// charge stays inside what its plugin was left, what remains always covers
/// every plugin's unspent share: no charge by one plugin can reduce what
/// another plugin's reservation still guarantees it.
#[derive(Clone, Debug, Default)]
pub(crate) struct KnowledgeBudgetLedger {
    reservations: KnowledgePublicationReservations,
    used: KnowledgeBudgetUsage,
    /// What each plugin with a reservation has taken, keyed by plugin.
    reserved_used: BTreeMap<String, KnowledgeBudgetUsage>,
}

impl KnowledgeBudgetLedger {
    pub(crate) fn new(reservations: KnowledgePublicationReservations) -> Self {
        Self {
            reservations,
            used: KnowledgeBudgetUsage::default(),
            reserved_used: BTreeMap::new(),
        }
    }

    /// The budget a system of `plugin` reads: the limits less what every
    /// publication charged so far took, less what the other plugins'
    /// reservations still hold.
    pub(crate) fn remaining_for(&self, plugin: &str) -> KnowledgePublicationBudget {
        let mut withheld = KnowledgeBudgetUsage::default();
        for (holder, reservation) in &self.reservations {
            if holder == plugin {
                continue;
            }
            let spent = self.reserved_used.get(holder).copied().unwrap_or_default();
            withheld = withheld.plus(KnowledgeBudgetUsage {
                records: reservation.records.saturating_sub(spent.records),
                entries: reservation.holder_entries.saturating_sub(spent.entries),
                payload_bytes: reservation
                    .payload_bytes
                    .saturating_sub(spent.payload_bytes),
            });
        }
        self.used.remaining_less(withheld)
    }

    /// Charges what one proposal, or one directive, of `plugin` publishes;
    /// refused past what [`Self::remaining_for`] left the plugin.
    pub(crate) fn charge(
        &mut self,
        plugin: &str,
        usage: KnowledgeBudgetUsage,
    ) -> Result<(), CanwuError> {
        let remaining = self.remaining_for(plugin);
        if usage.records > remaining.records
            || usage.entries > remaining.holder_entries
            || usage.payload_bytes > remaining.payload_bytes
        {
            return Err(CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "boundary knowledge publication budget exceeded: the system published past what knowledge_publication_budget left",
            ));
        }
        self.used = self.used.plus(usage);
        if self.reservations.contains_key(plugin) {
            let spent = self.reserved_used.entry(plugin.to_owned()).or_default();
            *spent = spent.plus(usage);
        }
        Ok(())
    }
}

impl From<KnowledgeEncodingError> for CanwuError {
    fn from(error: KnowledgeEncodingError) -> Self {
        Self::new(ErrorCode::InvalidKnowledgeRecord, error.message())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "kind", rename_all = "snake_case")]
pub enum KnowledgeSubjectTargetKind {
    Core(CoreEntityKind),
    Domain(DomainRecordKind),
    AnyEntity,
    Event,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeSubjectSchema {
    pub role: String,
    pub targets: Vec<KnowledgeSubjectTargetKind>,
    pub required: bool,
    pub multiple: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginKnowledgeSchema {
    pub id: KnowledgeSchemaId,
    pub schema_hash: String,
    pub writable: bool,
    pub payload_schema: PayloadSchema,
    pub subjects: Vec<KnowledgeSubjectSchema>,
    /// When an entry of this schema leaves its holder. Part of the plugin
    /// descriptor: a run restores and replays only with a plugin that
    /// declares the same policy. Under
    /// [`KnowledgeRetention::RetireWhenSuperseded`] an entry a new entry of
    /// the same holder supersedes is retired in the commit of the boundary
    /// that publishes the new one, live and in replay alike.
    pub retention: KnowledgeRetention,
}

impl PluginKnowledgeSchema {
    pub(crate) fn canonicalize(&mut self) {
        for subject in &mut self.subjects {
            subject.targets.sort();
            subject.targets.dedup();
        }
        self.subjects
            .sort_by(|left, right| left.role.cmp(&right.role));
    }

    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        validate_kind(&self.id.kind)?;
        if self.id.version == 0 {
            return Err(invalid_schema("knowledge schema versions must be nonzero"));
        }
        if !is_canonical_hash(&self.schema_hash) {
            return Err(invalid_schema(
                "knowledge schemas require a canonical semantic hash",
            ));
        }
        if self
            .subjects
            .windows(2)
            .any(|pair| pair[0].role >= pair[1].role)
        {
            return Err(invalid_schema(
                "knowledge subject roles must be unique and sorted",
            ));
        }
        for subject in &self.subjects {
            if !canonical_text(&subject.role)
                || subject.targets.is_empty()
                || subject.targets.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid_schema(
                    "knowledge subjects require canonical roles and unique sorted targets",
                ));
            }
            for target in &subject.targets {
                if let KnowledgeSubjectTargetKind::Domain(kind) = target {
                    validate_domain_kind(kind)?;
                }
            }
        }
        Ok(())
    }
}

pub(crate) type KnowledgeSchemas = BTreeMap<KnowledgeSchemaId, (String, PluginKnowledgeSchema)>;
pub(crate) type KnowledgeKindOwners = BTreeMap<KnowledgeRecordKind, String>;

/// The retention a registered schema declares; `Keep` for one that is not
/// registered.
pub(crate) fn schema_retention(
    schemas: &KnowledgeSchemas,
    schema: &KnowledgeSchemaId,
) -> KnowledgeRetention {
    schemas
        .get(schema)
        .map_or(KnowledgeRetention::Keep, |(_, schema)| schema.retention)
}

/// Why a record may not supersede or contradict a named target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KnowledgeRelationError {
    /// The target is not in the holder's ledger at the publisher's cut.
    Unresolved,
    /// The target has a different schema kind from the record naming it.
    CrossKind,
}

/// The one rule for the entries a published entry may supersede or
/// contradict, shared by settlement and by load validation.
///
/// Every target must be an entry of the same `holder`, of the same schema
/// `kind`, that is visible at the publisher's cut:
///
/// - `committed`: the holder's entries in the ledgers committed by earlier
///   boundaries, whatever their learned time, so a target learned at the same
///   instant in an earlier boundary resolves; or
/// - `visible`: the entries the settling boundary staged for the holder with
///   same-boundary visibility in a phase before the publisher's.
///
/// An entry of the publisher's own phase, an entry staged for the next
/// boundary, a record that does not exist yet, and an entry of another holder
/// are in neither set. Record IDs are allocated in staging order, so every
/// admitted target has a lower ID than the record naming it: a relation can
/// never point forward, at itself, or around a cycle.
///
/// Settlement passes the committed ledgers, and load validation the ledgers
/// it has rebuilt from boundary evidence so far.
pub(crate) fn validate_relation_targets<'a>(
    holder: &KnowledgeHolderRef,
    kind: &KnowledgeRecordKind,
    related: impl IntoIterator<Item = &'a KnowledgeRecordId>,
    committed: &HolderLedgers,
    visible: &StagedKnowledgeEntries,
) -> Result<(), KnowledgeRelationError> {
    let committed = committed.get(holder);
    let visible = visible.get(holder);
    for id in related {
        let Some(target) = committed.and_then(|ledger| ledger.body(id)).or_else(|| {
            visible
                .and_then(|entries| entries.get(id))
                .map(|(body, _)| body)
        }) else {
            return Err(KnowledgeRelationError::Unresolved);
        };
        if target.schema.kind != *kind {
            return Err(KnowledgeRelationError::CrossKind);
        }
    }
    Ok(())
}

/// A holder's current records of one schema kind at a read cut, as returned
/// by [`crate::SimulationView::knowledge_heads`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeHeads {
    /// Global record IDs in ascending order. When more than
    /// [`KnowledgeLimitsV1::relations_per_record`] records match, the number
    /// one record may supersede, these are the newest of them.
    pub records: Vec<KnowledgeRecordId>,
    /// Whether more current records matched than `records` holds. The older
    /// ones that were left out stay current.
    pub truncated: bool,
}

/// Returns the holder's records of `kind` that carry every one of `subjects`
/// and that no entry of the holder supersedes, over the same two sets the
/// relation rule reads: the ledgers `committed` by earlier boundaries and the
/// entries `visible` at the reader's cut. Every returned ID is therefore a
/// target [`validate_relation_targets`] admits for the same holder and kind.
///
/// The newest `limit` matches are returned, in ascending ID order. Records of
/// the settling boundary have higher IDs than committed ones, so the visible
/// entries are read first, from the highest ID down, then the committed ones,
/// and the result is reversed.
///
/// The committed side reads the holder ledger's index of current records,
/// never its superseded records: it walks, newest first, the current records
/// of `kind` that carry the listed subject the fewest of them carry, skips
/// those a visible entry supersedes, and stops at `limit + 1` matches. The
/// visible side is the holder's entries of this boundary, which the boundary
/// itself staged. A relation never crosses kinds, so only visible entries of
/// `kind` can supersede a record of `kind`.
pub(crate) fn current_heads_at_cut(
    holder: &KnowledgeHolderRef,
    kind: &KnowledgeRecordKind,
    subjects: &[KnowledgeSubject],
    committed: &HolderLedgers,
    visible: Option<&StagedKnowledgeEntries>,
    limit: usize,
) -> KnowledgeHeads {
    let staged: Vec<_> = visible
        .and_then(|entries| entries.get(holder))
        .into_iter()
        .flat_map(BTreeMap::values)
        .filter(|(body, _)| body.schema.kind == *kind)
        .collect();
    let superseded_at_cut: std::collections::BTreeSet<KnowledgeRecordId> = staged
        .iter()
        .flat_map(|(_, entry)| entry.supersedes.iter().copied())
        .collect();
    let carries_subjects = |body: &KnowledgeRecordBody| {
        subjects
            .iter()
            .all(|subject| body.subjects.contains(subject))
    };
    let staged_heads = staged
        .iter()
        .rev()
        .map(|(body, _)| body.as_ref())
        .filter(|body| !superseded_at_cut.contains(&body.id) && carries_subjects(body));
    let committed_heads = committed
        .get(holder)
        .into_iter()
        .flat_map(|ledger| ledger.current_records(kind, subjects))
        .filter(|body| !superseded_at_cut.contains(&body.id));
    let mut heads: Vec<KnowledgeRecordId> = staged_heads
        .chain(committed_heads)
        .map(|body| body.id)
        .take(limit.saturating_add(1))
        .collect();
    let truncated = heads.len() > limit;
    heads.truncate(limit);
    heads.reverse();
    KnowledgeHeads {
        records: heads,
        truncated,
    }
}

pub(crate) fn validate_schema_set(
    schemas: &KnowledgeSchemas,
    owners: &KnowledgeKindOwners,
) -> Result<(), CanwuError> {
    let mut writable = BTreeMap::<KnowledgeRecordKind, usize>::new();
    let mut retention = BTreeMap::<&KnowledgeRecordKind, KnowledgeRetention>::new();
    for (id, (owner, schema)) in schemas {
        if id != &schema.id || owners.get(&id.kind) != Some(owner) {
            return Err(invalid_schema(
                "knowledge schema registry ownership is inconsistent",
            ));
        }
        schema.validate()?;
        if schema.writable {
            *writable.entry(id.kind.clone()).or_default() += 1;
        }
        // Retirement follows the superseding entry's schema, and a kind's
        // entries supersede across its versions, so every version of a kind
        // retires alike.
        if *retention.entry(&id.kind).or_insert(schema.retention) != schema.retention {
            return Err(invalid_schema(
                "every version of a knowledge kind must declare the same retention",
            ));
        }
    }
    if owners.keys().any(|kind| {
        !schemas.keys().any(|schema| &schema.kind == kind)
            || writable.get(kind).copied().unwrap_or_default() != 1
    }) {
        return Err(invalid_schema(
            "each knowledge kind requires exactly one writable version",
        ));
    }
    Ok(())
}

/// The fields of a body or draft that its shape is checked on.
pub(crate) struct BodyShape<'a> {
    pub(crate) schema: &'a KnowledgeSchemaId,
    pub(crate) subjects: &'a [KnowledgeSubject],
    pub(crate) payload: &'a Value,
    pub(crate) confidence_per_mille: u16,
    pub(crate) origin: &'a KnowledgeOrigin,
}

impl<'a> BodyShape<'a> {
    pub(crate) const fn of_draft(draft: &'a KnowledgeRecordDraft) -> Self {
        Self {
            schema: &draft.schema,
            subjects: draft.subjects.as_slice(),
            payload: &draft.payload,
            confidence_per_mille: draft.confidence_per_mille,
            origin: &draft.origin,
        }
    }

    pub(crate) const fn of_body(body: &'a KnowledgeRecordBody) -> Self {
        Self {
            schema: &body.schema,
            subjects: body.subjects.as_slice(),
            payload: &body.payload,
            confidence_per_mille: body.confidence_per_mille,
            origin: &body.origin,
        }
    }
}

/// Validates one draft for publication to one holder: the holder, the
/// body's shape, and the draft's relation lists. Settlement checks the same
/// things piecewise, once per body and once per holder.
#[cfg(test)]
pub(crate) fn validate_draft(
    draft: &KnowledgeRecordDraft,
    schema: &PluginKnowledgeSchema,
    holder: &KnowledgeHolderRef,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    validate_holder_for_publication(holder, current, record_schemas)?;
    validate_body_shape(
        &BodyShape::of_draft(draft),
        draft.payload_bytes()?,
        schema,
        current,
        record_schemas,
        true,
    )?;
    validate_relation_lists(&draft.supersedes, &draft.contradicts)
}

/// Validates a body's shape against its schema: the schema version, its
/// writability when publishing, confidence, origin method and evidence,
/// subjects, and the payload, whose encoding took `payload_bytes`.
pub(crate) fn validate_body_shape(
    body: &BodyShape<'_>,
    payload_bytes: usize,
    schema: &PluginKnowledgeSchema,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
    require_writable: bool,
) -> Result<(), CanwuError> {
    if body.schema != &schema.id {
        return Err(invalid_knowledge(
            "knowledge draft names the wrong schema version",
        ));
    }
    if require_writable && !schema.writable {
        return Err(invalid_knowledge(
            "knowledge draft uses a read-only schema version",
        ));
    }
    if body.confidence_per_mille > 1_000 {
        return Err(invalid_knowledge(
            "knowledge confidence exceeds 1000 per mille",
        ));
    }
    if !canonical_text(&body.origin.method)
        || body.origin.method.len() > KnowledgeLimitsV1::CURRENT.text_bytes
    {
        return Err(invalid_knowledge(
            "knowledge origin method is not canonical or exceeds its limit",
        ));
    }
    if body.subjects.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
        || body.origin.evidence.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
    {
        return Err(invalid_knowledge(
            "knowledge record relation limit exceeded",
        ));
    }
    if !strictly_sorted(body.subjects) || !strictly_sorted(&body.origin.evidence) {
        return Err(invalid_knowledge(
            "knowledge subjects and evidence references must be sorted and unique",
        ));
    }
    if payload_bytes > KnowledgeLimitsV1::CURRENT.payload_bytes_per_record {
        return Err(invalid_knowledge("knowledge payload byte limit exceeded"));
    }
    schema.payload_schema.validate(body.payload)?;
    validate_subjects(body.subjects, schema, current, record_schemas)?;
    Ok(())
}

/// Validates one entry's relation lists: each within the relation limit,
/// sorted, unique, and disjoint from the other.
pub(crate) fn validate_relation_lists(
    supersedes: &[KnowledgeRecordId],
    contradicts: &[KnowledgeRecordId],
) -> Result<(), CanwuError> {
    if supersedes.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
        || contradicts.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
    {
        return Err(invalid_knowledge(
            "knowledge record relation limit exceeded",
        ));
    }
    if !strictly_sorted(supersedes)
        || !strictly_sorted(contradicts)
        || supersedes.iter().any(|id| contradicts.contains(id))
    {
        return Err(invalid_knowledge(
            "knowledge supersedes and contradicts relations must be sorted, unique, and disjoint",
        ));
    }
    Ok(())
}

/// Validates the committed generic ledger of a snapshot or continuation
/// checkpoint and returns its highest body ID. Each body's shape is checked
/// once, however many holders hold it; each holder is checked once; each
/// entry's relations must be valid lists naming entries of the same holder
/// and kind. An entry of a schema whose retention is
/// [`KnowledgeRetention::RetireWhenSuperseded`] cannot still supersede
/// anything: the commit that published it retired its targets and dropped
/// their names, so a retained name is a tampered ledger.
pub(crate) fn validate_snapshot_records(
    snapshot: &SimulationSnapshot,
    schemas: &KnowledgeSchemas,
    record_schemas: &DomainRecordSchemas,
) -> Result<u64, CanwuError> {
    let current = RuntimeCurrentState {
        entities: snapshot.entities.iter().cloned().collect(),
        people: snapshot
            .world
            .people
            .iter()
            .cloned()
            .map(|value| (value.id, std::sync::Arc::new(value)))
            .collect(),
        person_availability: snapshot
            .person_availability
            .iter()
            .map(|(person, availability)| (*person, availability.clone()))
            .collect(),
        created_persons: snapshot.created_persons.iter().cloned().collect(),
        letters: snapshot
            .world
            .letters
            .iter()
            .cloned()
            .map(|value| (value.id, std::sync::Arc::new(value)))
            .collect(),
        governments: snapshot
            .world
            .governments
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        territories: snapshot
            .world
            .territories
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        routes: snapshot
            .world
            .routes
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        armies: snapshot
            .world
            .armies
            .iter()
            .cloned()
            .map(|value| (value.id, std::sync::Arc::new(value)))
            .collect(),
        knowledge: std::sync::Arc::new(snapshot.knowledge.clone()),
        plugin_components: snapshot
            .plugin_components
            .iter()
            .cloned()
            .map(|record| {
                (
                    std::sync::Arc::new(component_key(
                        &record.plugin,
                        &record.state,
                        &record.entity,
                        &record.component,
                    )),
                    std::sync::Arc::new(record),
                )
            })
            .collect(),
        domain_records: PersistentDomainRecordStore::from_records(
            snapshot
                .domain_records
                .iter()
                .cloned()
                .map(|record| (record.reference.clone(), record))
                .collect(),
        )?,
        decisions: snapshot.decisions.clone(),
        root_seed: snapshot.root_seed,
        authority_root_seed: snapshot.authority_root_seed,
        random_streams: snapshot
            .random_streams
            .iter()
            .cloned()
            .map(|stream| (stream.key.clone(), stream))
            .collect(),
    };
    let ledgers = &snapshot.knowledge.records;
    let mut max_id = 0_u64;
    for body in ledgers.bodies() {
        if body.learned_at > snapshot.now {
            return Err(invalid_knowledge(
                "knowledge records cannot be learned after the snapshot time",
            ));
        }
        let Some((_, schema)) = schemas.get(&body.schema) else {
            return Err(invalid_knowledge(
                "knowledge record uses an unregistered schema",
            ));
        };
        let payload_bytes = body_payload_bytes(body)?;
        validate_body_shape(
            &BodyShape::of_body(body),
            payload_bytes,
            schema,
            &current,
            record_schemas,
            false,
        )?;
        max_id = max_id.max(body.id.get());
    }
    for (holder, ledger) in ledgers {
        validate_historical_holder(holder, &current, record_schemas)?;
        for (entry, body) in ledger.entries() {
            validate_relation_lists(&entry.supersedes, &entry.contradicts)?;
            if entry
                .supersedes
                .iter()
                .chain(&entry.contradicts)
                .any(|target| {
                    ledger
                        .body(target)
                        .is_none_or(|target| target.schema.kind != body.schema.kind)
                })
            {
                return Err(invalid_knowledge(
                    "knowledge relations must resolve within the same holder ledger and kind",
                ));
            }
            if !entry.supersedes.is_empty()
                && schema_retention(schemas, &body.schema)
                    == KnowledgeRetention::RetireWhenSuperseded
            {
                return Err(invalid_knowledge(
                    "a knowledge entry of a retiring schema still names what its publication retired",
                ));
            }
        }
    }
    Ok(max_id)
}

/// The payload bytes of a stored body, measured as a publication of it was
/// charged.
pub(crate) fn body_payload_bytes(body: &KnowledgeRecordBody) -> Result<usize, CanwuError> {
    Ok(body.payload_bytes()?)
}

pub(crate) fn validate_holder_for_publication(
    holder: &KnowledgeHolderRef,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    let allowed = match holder {
        KnowledgeHolderRef::Person(id) => {
            current.entities.contains(&EntityRef::Person(*id)) || current.people.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Army(id)) => {
            current.entities.contains(&EntityRef::Army(*id)) || current.armies.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Government(id)) => {
            current.entities.contains(&EntityRef::Government(*id))
                || current.governments.contains_key(id)
        }
        KnowledgeHolderRef::Entity(
            EntityRef::Organization(_)
            | EntityRef::Person(_)
            | EntityRef::Resource(_)
            | EntityRef::Route(_)
            | EntityRef::Territory(_),
        ) => false,
        KnowledgeHolderRef::Entity(EntityRef::Domain(reference)) => {
            current.domain_records.get(reference).is_some_and(|record| {
                !record.is_deleted()
                    && record.is_active()
                    && record.class == DomainRecordClass::Entity
                    && record_schemas
                        .get(&reference.kind)
                        .is_some_and(|(_, schema)| {
                            schema.holder_policy == KnowledgeHolderPolicy::Allowed
                        })
            })
        }
    };
    if !allowed {
        return Err(CanwuError::new(
            ErrorCode::InvalidKnowledgeHolder,
            "knowledge holder is missing, retired, deleted, duplicated as a person entity, or ineligible",
        ));
    }
    if let KnowledgeHolderRef::Person(id) = holder
        && current
            .person_availability
            .get(id)
            .is_some_and(|availability| availability.life == super::LifeState::Dead)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidKnowledgeHolder,
            "knowledge cannot be published to a dead person's holder ledger",
        ));
    }
    Ok(())
}

fn validate_historical_holder(
    holder: &KnowledgeHolderRef,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    let allowed = match holder {
        KnowledgeHolderRef::Person(id) => {
            current.entities.contains(&EntityRef::Person(*id)) || current.people.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Army(id)) => {
            current.entities.contains(&EntityRef::Army(*id)) || current.armies.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Government(id)) => {
            current.entities.contains(&EntityRef::Government(*id))
                || current.governments.contains_key(id)
        }
        KnowledgeHolderRef::Entity(
            EntityRef::Organization(_)
            | EntityRef::Person(_)
            | EntityRef::Resource(_)
            | EntityRef::Route(_)
            | EntityRef::Territory(_),
        ) => false,
        KnowledgeHolderRef::Entity(EntityRef::Domain(reference)) => {
            current.domain_records.get(reference).is_some_and(|record| {
                record.class == DomainRecordClass::Entity
                    && record_schemas
                        .get(&reference.kind)
                        .is_some_and(|(_, schema)| {
                            schema.holder_policy == KnowledgeHolderPolicy::Allowed
                        })
            })
        }
    };
    if !allowed {
        return Err(CanwuError::new(
            ErrorCode::InvalidKnowledgeHolder,
            "knowledge record names an unknown or ineligible historical holder",
        ));
    }
    Ok(())
}

fn validate_subjects(
    subjects: &[KnowledgeSubject],
    schema: &PluginKnowledgeSchema,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for subject in subjects {
        if !canonical_text(&subject.role) {
            return Err(invalid_knowledge(
                "knowledge subject roles must be canonical",
            ));
        }
        let Some(declaration) = schema
            .subjects
            .iter()
            .find(|item| item.role == subject.role)
        else {
            return Err(invalid_knowledge(
                "knowledge record contains an undeclared subject role",
            ));
        };
        if !subject_matches(subject, declaration, current, record_schemas) {
            return Err(invalid_knowledge(
                "knowledge subject target does not match its schema role",
            ));
        }
        let count = counts.entry(&subject.role).or_default();
        *count += 1;
        if !declaration.multiple && *count > 1 {
            return Err(invalid_knowledge(
                "knowledge singleton subject role is repeated",
            ));
        }
    }
    if schema.subjects.iter().any(|declaration| {
        declaration.required
            && counts
                .get(declaration.role.as_str())
                .copied()
                .unwrap_or_default()
                == 0
    }) {
        return Err(invalid_knowledge(
            "knowledge record is missing a required subject role",
        ));
    }
    Ok(())
}

fn subject_matches(
    subject: &KnowledgeSubject,
    schema: &KnowledgeSubjectSchema,
    current: &RuntimeCurrentState,
    _record_schemas: &DomainRecordSchemas,
) -> bool {
    match &subject.target {
        KnowledgeSubjectTarget::Event(_) => {
            schema.targets.contains(&KnowledgeSubjectTargetKind::Event)
        }
        KnowledgeSubjectTarget::Entity(entity) => {
            let exact = entity.core_kind().is_some_and(|kind| {
                schema
                    .targets
                    .contains(&KnowledgeSubjectTargetKind::Core(kind))
            }) && entity_exists(current, entity);
            let any = schema
                .targets
                .contains(&KnowledgeSubjectTargetKind::AnyEntity)
                && entity_exists(current, entity);
            exact || any
        }
        KnowledgeSubjectTarget::DomainRecord(reference) => {
            let exact = schema
                .targets
                .contains(&KnowledgeSubjectTargetKind::Domain(reference.kind.clone()));
            let any = schema
                .targets
                .contains(&KnowledgeSubjectTargetKind::AnyEntity)
                && current.domain_records.get(reference).is_some_and(|record| {
                    !record.is_deleted() && record.class == DomainRecordClass::Entity
                });
            (exact || any) && current.domain_records.contains_key(reference)
        }
    }
}

fn entity_exists(current: &RuntimeCurrentState, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Army(id) => current.entities.contains(entity) || current.armies.contains_key(id),
        EntityRef::Government(id) => {
            current.entities.contains(entity) || current.governments.contains_key(id)
        }
        EntityRef::Organization(_) | EntityRef::Resource(_) => current.entities.contains(entity),
        EntityRef::Person(id) => {
            current.entities.contains(entity) || current.people.contains_key(id)
        }
        EntityRef::Domain(reference) => {
            current.domain_records.get(reference).is_some_and(|record| {
                !record.is_deleted() && record.class == DomainRecordClass::Entity
            })
        }
        EntityRef::Route(id) => {
            current.entities.contains(entity) || current.routes.contains_key(id)
        }
        EntityRef::Territory(id) => {
            current.entities.contains(entity) || current.territories.contains_key(id)
        }
    }
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_kind(kind: &KnowledgeRecordKind) -> Result<(), CanwuError> {
    if !canonical_text(&kind.namespace) || !canonical_text(&kind.name) {
        return Err(invalid_schema("knowledge schema kind must be canonical"));
    }
    Ok(())
}

fn validate_domain_kind(kind: &DomainRecordKind) -> Result<(), CanwuError> {
    if !canonical_text(&kind.namespace) || !canonical_text(&kind.name) {
        return Err(invalid_schema(
            "knowledge subject domain kind must be canonical",
        ));
    }
    Ok(())
}

fn invalid_schema(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidKnowledgeSchema, message)
}

fn invalid_knowledge(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidKnowledgeRecord, message)
}

#[cfg(test)]
mod tests {
    use super::super::records::{DomainRecord, DomainRecordLifecycle, DomainRecordSchema};
    use super::*;
    use crate::DecisionState;
    use crate::runtime::{Government, MapPoint, Route, Territory};
    use canwu_core::HolderKnowledgeRecordId;
    use canwu_core::{
        DomainRecordRef, EventId, EvidenceRef, GovernmentId, KnowledgeRecordId, PersonId,
        ResourceId, RouteId, TerritoryId,
    };
    use canwu_knowledge::{HolderEntry, KnowledgeOrigin, KnowledgeRecord};
    use canwu_time::SimTime;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    /// A record staged by the settling boundary, as the overlay keeps it.
    fn staged(
        (id, record): (KnowledgeRecordId, KnowledgeRecord),
        local: u64,
    ) -> (KnowledgeRecordId, (Arc<KnowledgeRecordBody>, HolderEntry)) {
        let (_, body, supersedes, contradicts) = record.into_parts();
        (
            id,
            (
                Arc::new(body),
                HolderEntry {
                    record: id,
                    local: HolderKnowledgeRecordId::new(local),
                    supersedes,
                    contradicts,
                },
            ),
        )
    }

    fn knowledge_kind() -> KnowledgeRecordKind {
        KnowledgeRecordKind::new("fixture.knowledge", "assessment")
    }

    fn knowledge_schema(subjects: Vec<KnowledgeSubjectSchema>) -> PluginKnowledgeSchema {
        PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), 1),
            schema_hash: "1000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects,
            retention: KnowledgeRetention::Keep,
        }
    }

    fn current_state(domain_records: Vec<DomainRecord>) -> RuntimeCurrentState {
        RuntimeCurrentState {
            entities: im::OrdSet::new(),
            people: im::OrdMap::new(),
            person_availability: im::OrdMap::new(),
            created_persons: im::Vector::new(),
            letters: im::OrdMap::new(),
            governments: im::OrdMap::new(),
            territories: im::OrdMap::new(),
            routes: im::OrdMap::new(),
            armies: im::OrdMap::new(),
            knowledge: std::sync::Arc::default(),
            plugin_components: im::OrdMap::new(),
            domain_records: PersistentDomainRecordStore::from_records(
                domain_records
                    .into_iter()
                    .map(|record| (record.reference.clone(), record))
                    .collect(),
            )
            .unwrap(),
            decisions: DecisionState::default(),
            root_seed: 1,
            authority_root_seed: 1,
            random_streams: im::OrdMap::new(),
        }
    }

    fn domain_record(
        reference: DomainRecordRef,
        class: DomainRecordClass,
        lifecycle: DomainRecordLifecycle,
    ) -> DomainRecord {
        DomainRecord {
            reference,
            owner: "fixture-domain".to_owned(),
            class,
            version: 1,
            lifecycle,
            payload: json!(null),
            references: vec![],
        }
    }

    fn holder_schemas(
        kind: &DomainRecordKind,
        policy: KnowledgeHolderPolicy,
    ) -> DomainRecordSchemas {
        let mut schema = DomainRecordSchema::new(kind.clone(), DomainRecordClass::Entity);
        schema.holder_policy = policy;
        BTreeMap::from([(kind.clone(), ("fixture-domain".to_owned(), schema))])
    }

    fn base_draft() -> KnowledgeRecordDraft {
        KnowledgeRecordDraft {
            schema: KnowledgeSchemaId::new(knowledge_kind(), 1),
            subjects: vec![],
            payload: json!(null),
            as_of: None,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: "fixture".to_owned(),
                evidence: vec![],
            },
            supersedes: vec![],
            contradicts: vec![],
        }
    }

    fn government_current() -> (KnowledgeHolderRef, RuntimeCurrentState) {
        let government = Government {
            id: GovernmentId::new(1),
            name: "Fixture".to_owned(),
            capital: TerritoryId::new(1),
        };
        let mut current = current_state(vec![]);
        current.governments.insert(government.id, government);
        (
            KnowledgeHolderRef::Entity(EntityRef::Government(GovernmentId::new(1))),
            current,
        )
    }

    #[test]
    fn holder_eligibility_distinguishes_publication_from_retained_history() {
        let kind = DomainRecordKind::new("fixture.organization", "office");
        let active_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "active");
        let retired_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "retired");
        let deleted_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "deleted");
        let current = current_state(vec![
            domain_record(
                active_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Active,
            ),
            domain_record(
                retired_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Retired {
                    at: SimTime::EPOCH,
                    successor: Some(active_ref.clone()),
                },
            ),
            domain_record(
                deleted_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Deleted { at: SimTime::EPOCH },
            ),
        ]);
        let schemas = holder_schemas(&kind, KnowledgeHolderPolicy::Allowed);
        let active = KnowledgeHolderRef::Entity(EntityRef::Domain(active_ref));
        let retired = KnowledgeHolderRef::Entity(EntityRef::Domain(retired_ref));
        let deleted = KnowledgeHolderRef::Entity(EntityRef::Domain(deleted_ref));

        assert!(validate_holder_for_publication(&active, &current, &schemas).is_ok());
        assert!(validate_holder_for_publication(&retired, &current, &schemas).is_err());
        assert!(validate_holder_for_publication(&deleted, &current, &schemas).is_err());
        assert!(validate_historical_holder(&retired, &current, &schemas).is_ok());
        assert!(validate_historical_holder(&deleted, &current, &schemas).is_ok());

        let disallowed = holder_schemas(&kind, KnowledgeHolderPolicy::Disallowed);
        assert!(validate_holder_for_publication(&active, &current, &disallowed).is_err());
        for holder in [
            KnowledgeHolderRef::Entity(EntityRef::Person(PersonId::new(1))),
            KnowledgeHolderRef::Entity(EntityRef::Resource(ResourceId::new(1))),
            KnowledgeHolderRef::Entity(EntityRef::Route(RouteId::new(1))),
            KnowledgeHolderRef::Entity(EntityRef::Territory(TerritoryId::new(1))),
        ] {
            let error = validate_holder_for_publication(&holder, &current, &schemas)
                .expect_err("ineligible core holder aliases must be rejected");
            assert_eq!(error.code, ErrorCode::InvalidKnowledgeHolder);
        }
    }

    #[test]
    fn generic_person_registry_is_an_eligible_person_holder() {
        let person = PersonId::new(7);
        let holder = KnowledgeHolderRef::Person(person);
        let mut current = current_state(vec![]);
        current.entities.insert(EntityRef::Person(person));

        assert!(validate_holder_for_publication(&holder, &current, &BTreeMap::new()).is_ok());
        assert!(validate_historical_holder(&holder, &current, &BTreeMap::new()).is_ok());
    }

    #[test]
    fn any_entity_accepts_future_entity_kinds_but_rejects_value_records() {
        let entity_kind = DomainRecordKind::new("future.organization", "institution");
        let value_kind = DomainRecordKind::new("future.organization", "memo");
        let entity_ref = DomainRecordRef::new(&entity_kind.namespace, &entity_kind.name, "one");
        let value_ref = DomainRecordRef::new(&value_kind.namespace, &value_kind.name, "one");
        let mut current = current_state(vec![
            domain_record(
                entity_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Active,
            ),
            domain_record(
                value_ref.clone(),
                DomainRecordClass::Record,
                DomainRecordLifecycle::Active,
            ),
        ]);
        current.territories.insert(
            TerritoryId::new(1),
            Territory {
                id: TerritoryId::new(1),
                name: "Territory".to_owned(),
                controller: GovernmentId::new(1),
                position: MapPoint::default(),
            },
        );
        current.routes.insert(
            RouteId::new(1),
            Route {
                id: RouteId::new(1),
                name: "Route".to_owned(),
                from: TerritoryId::new(1),
                to: TerritoryId::new(2),
                travel_minutes: 1,
                terrain: "road".to_owned(),
            },
        );
        let schemas = holder_schemas(&entity_kind, KnowledgeHolderPolicy::Allowed);
        let schema = knowledge_schema(vec![KnowledgeSubjectSchema {
            role: "subject".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
            required: true,
            multiple: false,
        }]);
        let holder = KnowledgeHolderRef::Entity(EntityRef::Domain(entity_ref.clone()));
        let mut draft = base_draft();
        draft.subjects = vec![KnowledgeSubject {
            role: "subject".to_owned(),
            target: KnowledgeSubjectTarget::Entity(EntityRef::Domain(entity_ref)),
        }];
        assert!(validate_draft(&draft, &schema, &holder, &current, &schemas).is_ok());

        draft.subjects[0].target =
            KnowledgeSubjectTarget::Entity(EntityRef::Territory(TerritoryId::new(1)));
        assert!(validate_draft(&draft, &schema, &holder, &current, &schemas).is_ok());
        draft.subjects[0].target =
            KnowledgeSubjectTarget::Entity(EntityRef::Route(RouteId::new(1)));
        assert!(validate_draft(&draft, &schema, &holder, &current, &schemas).is_ok());

        draft.subjects[0].target = KnowledgeSubjectTarget::DomainRecord(value_ref);
        let error = validate_draft(&draft, &schema, &holder, &current, &schemas)
            .expect_err("AnyEntity must not accept ordinary domain value records");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord);

        let exact_person = knowledge_schema(vec![KnowledgeSubjectSchema {
            role: "subject".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Core(CoreEntityKind::Person)],
            required: true,
            multiple: false,
        }]);
        draft.subjects[0].target =
            KnowledgeSubjectTarget::Entity(EntityRef::Person(PersonId::new(999)));
        let error = validate_draft(&draft, &exact_person, &holder, &current, &schemas)
            .expect_err("an exact core target must still name an existing entity");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord);
    }

    #[test]
    fn schema_set_requires_exact_ownership_one_writable_version_and_one_retention() {
        let writable = knowledge_schema(vec![]);
        let read_only = PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), 2),
            writable: false,
            ..writable.clone()
        };
        let owners = BTreeMap::from([(knowledge_kind(), "fixture".to_owned())]);
        let schemas = BTreeMap::from([
            (
                writable.id.clone(),
                ("fixture".to_owned(), writable.clone()),
            ),
            (
                read_only.id.clone(),
                ("fixture".to_owned(), read_only.clone()),
            ),
        ]);
        assert!(validate_schema_set(&schemas, &owners).is_ok());

        let only_read_only =
            BTreeMap::from([(read_only.id.clone(), ("fixture".to_owned(), read_only))]);
        assert!(validate_schema_set(&only_read_only, &owners).is_err());

        let second_writable = PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), 2),
            ..writable.clone()
        };
        let two_writable = BTreeMap::from([
            (
                writable.id.clone(),
                ("fixture".to_owned(), writable.clone()),
            ),
            (
                second_writable.id.clone(),
                ("fixture".to_owned(), second_writable),
            ),
        ]);
        assert!(validate_schema_set(&two_writable, &owners).is_err());

        let retiring_read_only = PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), 2),
            writable: false,
            retention: KnowledgeRetention::RetireWhenSuperseded,
            ..writable.clone()
        };
        let mixed_retention = BTreeMap::from([
            (writable.id.clone(), ("fixture".to_owned(), writable)),
            (
                retiring_read_only.id.clone(),
                ("fixture".to_owned(), retiring_read_only),
            ),
        ]);
        let error = validate_schema_set(&mixed_retention, &owners)
            .expect_err("two versions of one kind must retire alike");
        assert!(error.message.contains("same retention"), "{error}");
    }

    #[test]
    fn current_heads_follow_the_cut_the_selection_and_the_bound() {
        let kind = knowledge_kind();
        let other_kind = KnowledgeRecordKind::new("fixture.knowledge", "rumour");
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let neighbour = KnowledgeHolderRef::Person(PersonId::new(2));
        let subject = |event: u64| KnowledgeSubject {
            role: "subject".to_owned(),
            target: KnowledgeSubjectTarget::Event(EventId::new(event)),
        };
        let record = |id: u64,
                      holder: &KnowledgeHolderRef,
                      kind: &KnowledgeRecordKind,
                      event: u64,
                      supersedes: &[u64]| {
            let draft = base_draft();
            let stored = KnowledgeRecord {
                id: KnowledgeRecordId::new(id),
                holder: holder.clone(),
                schema: KnowledgeSchemaId::new(kind.clone(), 1),
                subjects: vec![subject(event)],
                payload: draft.payload,
                as_of: None,
                learned_at: SimTime::EPOCH,
                confidence_per_mille: 1_000,
                origin: draft.origin,
                supersedes: supersedes
                    .iter()
                    .copied()
                    .map(KnowledgeRecordId::new)
                    .collect(),
                contradicts: Vec::new(),
            };
            (stored.id, stored)
        };
        let committed: HolderLedgers = [
            record(1, &holder, &kind, 7, &[]),
            record(2, &holder, &kind, 7, &[1]),
            record(3, &holder, &kind, 8, &[]),
            record(4, &holder, &other_kind, 7, &[]),
            record(5, &neighbour, &kind, 7, &[]),
        ]
        .into_iter()
        .map(|(_, stored)| stored)
        .collect();
        // Records the settling boundary staged for the same boundary.
        let visible: StagedKnowledgeEntries = BTreeMap::from([(
            holder.clone(),
            BTreeMap::from([
                staged(record(6, &holder, &kind, 8, &[3]), 5),
                staged(record(7, &holder, &kind, 9, &[]), 6),
            ]),
        )]);
        let heads = |holder: &KnowledgeHolderRef,
                     kind: &KnowledgeRecordKind,
                     subjects: &[KnowledgeSubject],
                     visible: Option<&StagedKnowledgeEntries>,
                     limit: usize| {
            let heads = current_heads_at_cut(holder, kind, subjects, &committed, visible, limit);
            (
                heads.records.iter().map(|id| id.get()).collect::<Vec<_>>(),
                heads.truncated,
            )
        };

        // Committed state alone: the superseded record is not current.
        assert_eq!(heads(&holder, &kind, &[], None, 64), (vec![2, 3], false));
        // A visible record of the boundary replaces the record it supersedes.
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 64),
            (vec![2, 6, 7], false)
        );
        // Subjects, kind, and holder select; none of them leaks across.
        assert_eq!(
            heads(&holder, &kind, &[subject(8)], Some(&visible), 64),
            (vec![6], false)
        );
        assert_eq!(
            heads(&holder, &kind, &[subject(7), subject(8)], None, 64),
            (vec![], false)
        );
        assert_eq!(heads(&holder, &other_kind, &[], None, 64), (vec![4], false));
        assert_eq!(
            heads(&neighbour, &kind, &[], Some(&visible), 64),
            (vec![5], false)
        );
        assert_eq!(
            heads(
                &KnowledgeHolderRef::Person(PersonId::new(3)),
                &kind,
                &[],
                None,
                64
            ),
            (vec![], false)
        );
        // The result is bounded: it keeps the newest records, in ascending
        // order, and says when older current records were left out.
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 3),
            (vec![2, 6, 7], false)
        );
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 2),
            (vec![6, 7], true)
        );
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 1),
            (vec![7], true)
        );
        assert_eq!(heads(&holder, &kind, &[], None, 1), (vec![3], true));
    }

    /// `knowledge_heads` stays exact past the depth at which `im` 15.1's
    /// backward iteration drops elements: 5,000 current records of one
    /// holder, even IDs on one line, odd IDs on another, odd IDs 2,051 to
    /// 2,071 on both.
    #[test]
    fn current_heads_stay_exact_in_a_deep_ledger() {
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let line = |event: u64| KnowledgeSubject {
            role: "line".to_owned(),
            target: KnowledgeSubjectTarget::Event(EventId::new(event)),
        };
        let committed: HolderLedgers = (1..=5_000_u64)
            .map(|id| {
                let mut subjects = Vec::new();
                if id.is_multiple_of(2) || (2_051..=2_071).contains(&id) {
                    subjects.push(line(1));
                }
                if !id.is_multiple_of(2) {
                    subjects.push(line(2));
                }
                let draft = base_draft();
                KnowledgeRecord {
                    id: KnowledgeRecordId::new(id),
                    holder: holder.clone(),
                    schema: KnowledgeSchemaId::new(knowledge_kind(), 1),
                    subjects,
                    payload: draft.payload,
                    as_of: None,
                    learned_at: SimTime::EPOCH,
                    confidence_per_mille: 1_000,
                    origin: draft.origin,
                    supersedes: Vec::new(),
                    contradicts: Vec::new(),
                }
            })
            .collect();
        let heads = current_heads_at_cut(
            &holder,
            &knowledge_kind(),
            &[line(1), line(2)],
            &committed,
            None,
            KnowledgeLimitsV1::CURRENT.relations_per_record,
        );
        assert_eq!(
            heads.records,
            (2_051..=2_071)
                .step_by(2)
                .map(KnowledgeRecordId::new)
                .collect::<Vec<_>>()
        );
        assert!(!heads.truncated);
        let newest = current_heads_at_cut(&holder, &knowledge_kind(), &[], &committed, None, 64);
        assert_eq!(
            newest.records,
            (4_937..=5_000)
                .map(KnowledgeRecordId::new)
                .collect::<Vec<_>>()
        );
        assert!(newest.truncated);
    }

    #[test]
    fn per_record_limits_accept_boundary_and_reject_boundary_plus_one() {
        let (holder, current) = government_current();
        let schemas = BTreeMap::new();
        let no_subjects = knowledge_schema(vec![]);

        let mut payload = base_draft();
        payload.payload =
            json!("a".repeat(KnowledgeLimitsV1::CURRENT.payload_bytes_per_record - 2));
        assert!(validate_draft(&payload, &no_subjects, &holder, &current, &schemas).is_ok());
        payload.payload =
            json!("a".repeat(KnowledgeLimitsV1::CURRENT.payload_bytes_per_record - 1));
        assert!(validate_draft(&payload, &no_subjects, &holder, &current, &schemas).is_err());

        let mut method = base_draft();
        method.origin.method = "m".repeat(KnowledgeLimitsV1::CURRENT.text_bytes);
        assert!(validate_draft(&method, &no_subjects, &holder, &current, &schemas).is_ok());
        method.origin.method.push('m');
        assert!(validate_draft(&method, &no_subjects, &holder, &current, &schemas).is_err());

        let mut evidence = base_draft();
        evidence.origin.evidence = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| EvidenceRef::Event(EventId::new(id as u64)))
            .collect();
        assert!(validate_draft(&evidence, &no_subjects, &holder, &current, &schemas).is_ok());
        evidence
            .origin
            .evidence
            .push(EvidenceRef::Event(EventId::new(
                KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
            )));
        assert!(validate_draft(&evidence, &no_subjects, &holder, &current, &schemas).is_err());

        let mut supersedes = base_draft();
        supersedes.supersedes = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| KnowledgeRecordId::new(id as u64))
            .collect();
        assert!(validate_draft(&supersedes, &no_subjects, &holder, &current, &schemas).is_ok());
        supersedes.supersedes.push(KnowledgeRecordId::new(
            KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
        ));
        assert!(validate_draft(&supersedes, &no_subjects, &holder, &current, &schemas).is_err());

        let mut contradicts = base_draft();
        contradicts.contradicts = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| KnowledgeRecordId::new(id as u64))
            .collect();
        assert!(validate_draft(&contradicts, &no_subjects, &holder, &current, &schemas).is_ok());
        contradicts.contradicts.push(KnowledgeRecordId::new(
            KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
        ));
        assert!(validate_draft(&contradicts, &no_subjects, &holder, &current, &schemas).is_err());

        let event_subject = knowledge_schema(vec![KnowledgeSubjectSchema {
            role: "subject".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Event],
            required: true,
            multiple: true,
        }]);
        let mut subjects = base_draft();
        subjects.subjects = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| KnowledgeSubject {
                role: "subject".to_owned(),
                target: KnowledgeSubjectTarget::Event(EventId::new(id as u64)),
            })
            .collect();
        assert!(validate_draft(&subjects, &event_subject, &holder, &current, &schemas).is_ok());
        subjects.subjects.push(KnowledgeSubject {
            role: "subject".to_owned(),
            target: KnowledgeSubjectTarget::Event(EventId::new(
                KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
            )),
        });
        assert!(validate_draft(&subjects, &event_subject, &holder, &current, &schemas).is_err());
    }
}
