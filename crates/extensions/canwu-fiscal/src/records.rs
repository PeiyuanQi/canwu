//! The fiscal records: how a [`FiscalState`] is partitioned into records, put
//! back together, and audited.
//!
//! A run keeps its fiscal state as records of the kinds of
//! [`FiscalRecordKind`]. No fiscal record references another, except the
//! root's reference to the create-only catalog, so a write of one record
//! makes the engine re-validate no other. The records are the partition of
//! one logical [`FiscalState`], whose [`FiscalState::validate`] defines
//! validity: [`FiscalState::into_records`] partitions an authored state,
//! [`FiscalState::from_records`] assembles one, and [`audit_fiscal_records`]
//! checks that a run's records are exactly the partition of the state they
//! assemble to.

use crate::FiscalArchiveHeadV1;
use crate::PLUGIN_NAME;
use crate::model::{
    CompiledFiscalCatalog, FISCAL_DIRECTORY_BUCKETS, FISCAL_INDEX_PAGE_SCOPES,
    FISCAL_REPORT_QUEUE_PAGES, FiscalCatalogRecord, FiscalJurisdiction, FiscalObserver,
    FiscalOffice, FiscalReportQueueEntry, FiscalRoot, FiscalScopeLedger, FiscalState, ROOT_ID,
    fiscal_catalog_reference, fiscal_office_key, invalid, office_binding_ids,
};
use canwu_api::{
    CanwuError, CanwuRun, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordRef, DomainRecordType, DomainRecordVersionSource,
    DomainReference, DomainReferenceTarget, DomainValueKindClass, EntityRef, EvidenceRef,
    KnowledgeHolderRef, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadRequiredEvidenceContinuationV1, TypedDomainRecordRef, canonical_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The reference role under which the root names the catalog.
pub(crate) const ROOT_CATALOG_ROLE: &str = "catalog";
/// The reference role under which an observer record names its holder.
pub(crate) const OBSERVER_HOLDER_ROLE: &str = "holder";

macro_rules! fiscal_record_type {
    ($(#[$doc:meta])* $name:ident, $payload:ty, $record:literal) => {
        $(#[$doc])*
        pub struct $name;

        impl DomainRecordType for $name {
            type Payload = $payload;
            type Class = DomainValueKindClass;

            const NAMESPACE: &'static str = "canwu.fiscal";
            const NAME: &'static str = $record;
        }
    };
}

fiscal_record_type!(
    /// `canwu.fiscal:root`, identity `primary`: the historical context, the
    /// approved execution-evidence kinds, and the report queue's index.
    FiscalRootRecord,
    FiscalRoot,
    "root"
);
fiscal_record_type!(
    /// `canwu.fiscal:archive`, identity `primary`: the fiscal archive head.
    FiscalArchiveRecord,
    FiscalArchiveHeadV1,
    "archive"
);
fiscal_record_type!(
    /// `canwu.fiscal:office`, identity [`fiscal_office_key`] of its
    /// institution.
    FiscalOfficeRecord,
    FiscalOffice,
    "office"
);
fiscal_record_type!(
    /// `canwu.fiscal:ledger`, identity the scope binding ID.
    FiscalLedgerRecord,
    FiscalScopeLedger,
    "ledger"
);
fiscal_record_type!(
    /// `canwu.fiscal:jurisdiction`, identity the catalog region ID.
    FiscalJurisdictionRecord,
    FiscalJurisdiction,
    "jurisdiction"
);
fiscal_record_type!(
    /// `canwu.fiscal:observer`, identity the observer binding ID. The
    /// binding's report lines are named on this record.
    FiscalObserverRecord,
    FiscalObserver,
    "observer"
);
fiscal_record_type!(
    /// `canwu.fiscal:directory`, identity `dir.` and three hexadecimal
    /// digits: the office of every binding, policy, and proposal ID whose
    /// digest names the bucket.
    FiscalDirectoryRecord,
    FiscalDirectoryBucket,
    "directory"
);
fiscal_record_type!(
    /// `canwu.fiscal:index`, create-only: the scopes of one office in rank
    /// order, or of one region in ID order, [`FISCAL_INDEX_PAGE_SCOPES`] to a
    /// page.
    FiscalIndexRecord,
    FiscalIndexPage,
    "index"
);
fiscal_record_type!(
    /// `canwu.fiscal:report-queue`, identity `queue.` and three hexadecimal
    /// digits: the report work owed to every observer whose binding ID's
    /// digest names the page.
    FiscalReportQueueRecord,
    FiscalReportQueuePage,
    "report-queue"
);

/// The kinds of the fiscal records, besides the create-only catalog.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FiscalRecordKind {
    Root,
    Archive,
    Office,
    Ledger,
    Jurisdiction,
    Observer,
    Directory,
    Index,
    ReportQueue,
}

impl FiscalRecordKind {
    /// Every kind, in this order.
    pub const ALL: [Self; 9] = [
        Self::Root,
        Self::Archive,
        Self::Office,
        Self::Ledger,
        Self::Jurisdiction,
        Self::Observer,
        Self::Directory,
        Self::Index,
        Self::ReportQueue,
    ];

    /// The record kind.
    #[must_use]
    pub fn kind(self) -> DomainRecordKind {
        match self {
            Self::Root => DomainRecordKind::for_type::<FiscalRootRecord>(),
            Self::Archive => DomainRecordKind::for_type::<FiscalArchiveRecord>(),
            Self::Office => DomainRecordKind::for_type::<FiscalOfficeRecord>(),
            Self::Ledger => DomainRecordKind::for_type::<FiscalLedgerRecord>(),
            Self::Jurisdiction => DomainRecordKind::for_type::<FiscalJurisdictionRecord>(),
            Self::Observer => DomainRecordKind::for_type::<FiscalObserverRecord>(),
            Self::Directory => DomainRecordKind::for_type::<FiscalDirectoryRecord>(),
            Self::Index => DomainRecordKind::for_type::<FiscalIndexRecord>(),
            Self::ReportQueue => DomainRecordKind::for_type::<FiscalReportQueueRecord>(),
        }
    }

    /// The fiscal kind of a record kind, if it is one; the catalog is not.
    #[must_use]
    pub fn of(kind: &DomainRecordKind) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.kind() == *kind)
    }
}

#[must_use]
pub fn fiscal_root_reference() -> TypedDomainRecordRef<FiscalRootRecord> {
    TypedDomainRecordRef::new(ROOT_ID)
}

#[must_use]
pub fn fiscal_archive_reference() -> TypedDomainRecordRef<FiscalArchiveRecord> {
    TypedDomainRecordRef::new(ROOT_ID)
}

#[must_use]
pub fn fiscal_office_reference(
    institution: &EntityRef,
) -> TypedDomainRecordRef<FiscalOfficeRecord> {
    TypedDomainRecordRef::new(fiscal_office_key(institution))
}

pub(crate) fn office_reference(key: &str) -> TypedDomainRecordRef<FiscalOfficeRecord> {
    TypedDomainRecordRef::new(key)
}

#[must_use]
pub fn fiscal_ledger_reference(scope_binding_id: &str) -> TypedDomainRecordRef<FiscalLedgerRecord> {
    TypedDomainRecordRef::new(scope_binding_id)
}

#[must_use]
pub fn fiscal_jurisdiction_reference(
    region_id: &str,
) -> TypedDomainRecordRef<FiscalJurisdictionRecord> {
    TypedDomainRecordRef::new(region_id)
}

#[must_use]
pub fn fiscal_observer_reference(
    observer_binding_id: &str,
) -> TypedDomainRecordRef<FiscalObserverRecord> {
    TypedDomainRecordRef::new(observer_binding_id)
}

pub(crate) fn directory_reference(bucket: &str) -> TypedDomainRecordRef<FiscalDirectoryRecord> {
    TypedDomainRecordRef::new(bucket)
}

pub(crate) fn index_reference(page: &str) -> TypedDomainRecordRef<FiscalIndexRecord> {
    TypedDomainRecordRef::new(page)
}

pub(crate) fn queue_reference(page: &str) -> TypedDomainRecordRef<FiscalReportQueueRecord> {
    TypedDomainRecordRef::new(page)
}

/// The first `digits` hexadecimal digits of a domain-separated digest of
/// `value`.
fn digest_prefix(domain: &str, value: &str, digits: usize) -> String {
    let digest = canonical_hash(domain, &value).unwrap_or_else(|_| "0".repeat(64));
    digest[..digits].to_owned()
}

/// Hexadecimal digits that name `buckets` buckets.
const fn bucket_digits(buckets: usize) -> usize {
    (buckets.trailing_zeros() as usize).div_ceil(4)
}

const _: () = assert!(FISCAL_DIRECTORY_BUCKETS.is_power_of_two());
const _: () = assert!(FISCAL_REPORT_QUEUE_PAGES.is_power_of_two());
const _: () = assert!(FISCAL_DIRECTORY_BUCKETS.trailing_zeros().trailing_zeros() >= 2);
const _: () = assert!(FISCAL_REPORT_QUEUE_PAGES.trailing_zeros().trailing_zeros() >= 2);

/// The directory bucket of a binding, policy, or proposal ID.
#[must_use]
pub(crate) fn directory_bucket(id: &str) -> String {
    format!(
        "dir.{}",
        digest_prefix(
            "canwu.fiscal.directory-bucket.v1",
            id,
            bucket_digits(FISCAL_DIRECTORY_BUCKETS)
        )
    )
}

/// The report-queue page of an observer binding ID.
#[must_use]
pub(crate) fn report_queue_page(observer_binding_id: &str) -> String {
    format!(
        "queue.{}",
        digest_prefix(
            "canwu.fiscal.report-queue-page.v1",
            observer_binding_id,
            bucket_digits(FISCAL_REPORT_QUEUE_PAGES)
        )
    )
}

/// One bucket of the binding directory: the office key of each binding or
/// policy ID, and of each proposal ID, whose digest names the bucket. A
/// request names only a binding or proposal ID; admission and settlement find
/// the office that holds it here, with one read.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalDirectoryBucket {
    /// Authority binding, audit binding, and appointment policy IDs, one
    /// namespace, to the office that holds each.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, String>,
    /// Auditor proposal IDs to the office of the audited institution.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub proposals: BTreeMap<String, String>,
}

impl FiscalDirectoryBucket {
    /// Whether the bucket names nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty() && self.proposals.is_empty()
    }
}

/// Whose scopes an index page lists.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum FiscalIndexOwner {
    /// An office, by its key: its scopes in rank order.
    Office(String),
    /// A catalog region: the scopes that name it, in ID order.
    Jurisdiction(String),
}

impl FiscalIndexOwner {
    /// The identity of page `page` of this owner's index.
    #[must_use]
    pub fn page_id(&self, page: u32) -> String {
        let owner = match self {
            Self::Office(key) => format!("office:{key}"),
            Self::Jurisdiction(region) => format!("jurisdiction:{region}"),
        };
        format!(
            "index.{}.{page}",
            digest_prefix("canwu.fiscal.index-owner.v1", &owner, 32)
        )
    }

    /// The page and place of the `rank`-th scope.
    #[must_use]
    pub fn locate(rank: u32) -> (u32, usize) {
        let per_page = u32::try_from(FISCAL_INDEX_PAGE_SCOPES).unwrap_or(u32::MAX);
        (
            rank / per_page,
            usize::try_from(rank % per_page).unwrap_or(usize::MAX),
        )
    }
}

/// One page of a static scope index.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalIndexPage {
    pub owner: FiscalIndexOwner,
    pub page: u32,
    pub scopes: Vec<String>,
}

/// One page of the report queue: the report work owed to each observer whose
/// binding ID's digest names the page.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportQueuePage {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, FiscalReportQueueEntry>,
}

pub(crate) fn root_draft(root: &FiscalRoot) -> Result<DomainRecordDraft, CanwuError> {
    let mut draft = DomainRecordDraft::from_typed(fiscal_root_reference(), root)?;
    draft.references = vec![DomainReference::from_typed(
        ROOT_CATALOG_ROLE,
        fiscal_catalog_reference(),
    )];
    Ok(draft)
}

pub(crate) fn archive_draft(
    archive: &FiscalArchiveHeadV1,
) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(fiscal_archive_reference(), archive)
}

pub(crate) fn office_draft(office: &FiscalOffice) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(fiscal_office_reference(&office.institution), office)
}

/// The draft of a ledger record. Its payload declares, as required evidence,
/// the versions its hot books cite that a boundary established: their
/// commutation quotes and their receipts' evidence. An archived book's
/// citations stop being declared; [`FiscalState::prepare_fiscal_archive`]
/// refuses a book whose citations a seal has not covered yet.
pub(crate) fn ledger_draft(ledger: &FiscalScopeLedger) -> Result<DomainRecordDraft, CanwuError> {
    let mut draft =
        DomainRecordDraft::from_typed(fiscal_ledger_reference(&ledger.scope.id), ledger)?;
    let continuation = ledger_continuation(ledger);
    draft
        .payload
        .as_object_mut()
        .ok_or_else(|| invalid("fiscal ledger payload is not an object"))?
        .insert(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            serde_json::to_value(continuation).map_err(|error| {
                invalid(format!(
                    "fiscal evidence continuation could not be encoded: {error}"
                ))
            })?,
        );
    Ok(draft)
}

/// The serialized bytes of a ledger record's payload, exactly as
/// [`ledger_draft`] writes it: the ledger, and its evidence continuation in a
/// field of its own. Settlement holds a ledger to its budget by this, and
/// activation measures the stored payload, so the two agree.
pub(crate) fn ledger_payload_bytes(ledger: &FiscalScopeLedger) -> Result<usize, CanwuError> {
    let continuation = ledger_continuation(ledger);
    // `,"<field>":<continuation>` inside the ledger's object.
    Ok(crate::settle::json_len(ledger)?
        + 2
        + crate::settle::json_len(&PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD)?
        + crate::settle::json_len(&continuation)?)
}

/// The versions a ledger's hot books cite that a boundary established, as the
/// evidence continuation its record declares.
fn ledger_continuation(ledger: &FiscalScopeLedger) -> PayloadRequiredEvidenceContinuationV1 {
    let dependencies = ledger
        .books
        .values()
        .flat_map(crate::model::FiscalCycleBook::citations)
        .filter(|reference| {
            matches!(
                reference.established_by,
                DomainRecordVersionSource::BoundaryChange { .. }
            )
        })
        .cloned()
        .map(EvidenceRef::DomainRecordVersion)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if dependencies.is_empty() {
        PayloadRequiredEvidenceContinuationV1::completed()
    } else {
        PayloadRequiredEvidenceContinuationV1::active(dependencies)
    }
}

pub(crate) fn jurisdiction_draft(
    jurisdiction: &FiscalJurisdiction,
) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(
        fiscal_jurisdiction_reference(&jurisdiction.region_id),
        jurisdiction,
    )
}

/// The draft of an observer record. It references the person or government
/// that holds the binding's report; the engine checks a record's references
/// against its entity registry when a run is created or restored and at
/// every write, so a binding whose holder does not exist is refused where it
/// enters a run.
pub(crate) fn observer_draft(observer: &FiscalObserver) -> Result<DomainRecordDraft, CanwuError> {
    let mut draft =
        DomainRecordDraft::from_typed(fiscal_observer_reference(&observer.binding.id), observer)?;
    let holder = match &observer.binding.knowledge_holder {
        KnowledgeHolderRef::Person(person) => Some(EntityRef::Person(*person)),
        KnowledgeHolderRef::Entity(entity @ EntityRef::Government(_)) => Some(entity.clone()),
        KnowledgeHolderRef::Entity(_) => None,
    };
    draft.references = holder
        .map(|holder| DomainReference {
            role: OBSERVER_HOLDER_ROLE.to_owned(),
            target: DomainReferenceTarget::Core(holder),
        })
        .into_iter()
        .collect();
    Ok(draft)
}

pub(crate) fn directory_draft(
    bucket: &str,
    entries: &FiscalDirectoryBucket,
) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(directory_reference(bucket), entries)
}

pub(crate) fn index_draft(page: &FiscalIndexPage) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(index_reference(&page.owner.page_id(page.page)), page)
}

pub(crate) fn queue_draft(
    page: &str,
    entries: &FiscalReportQueuePage,
) -> Result<DomainRecordDraft, CanwuError> {
    DomainRecordDraft::from_typed(queue_reference(page), entries)
}

/// The directory buckets a state derives, without empty ones.
pub(crate) fn directory_buckets(state: &FiscalState) -> BTreeMap<String, FiscalDirectoryBucket> {
    let mut buckets = BTreeMap::<String, FiscalDirectoryBucket>::new();
    for (key, office) in &state.offices {
        for id in office_binding_ids(office) {
            buckets
                .entry(directory_bucket(id))
                .or_default()
                .bindings
                .insert(id.clone(), key.clone());
        }
        for id in office.audit_proposals.keys() {
            buckets
                .entry(directory_bucket(id))
                .or_default()
                .proposals
                .insert(id.clone(), key.clone());
        }
    }
    buckets
}

/// The static index pages a state derives: each office's scopes in rank
/// order and each region's scopes in ID order.
pub(crate) fn index_pages(state: &FiscalState) -> Vec<FiscalIndexPage> {
    let mut owners = BTreeMap::<FiscalIndexOwner, Vec<(u32, &String)>>::new();
    for ledger in state.ledgers.values() {
        owners
            .entry(FiscalIndexOwner::Office(ledger.office.clone()))
            .or_default()
            .push((ledger.rank, &ledger.scope.id));
        owners
            .entry(FiscalIndexOwner::Jurisdiction(
                ledger.scope.jurisdiction_id.clone(),
            ))
            .or_default()
            .push((0, &ledger.scope.id));
    }
    let mut pages = Vec::new();
    for (owner, mut scopes) in owners {
        scopes.sort();
        for (page, chunk) in scopes.chunks(FISCAL_INDEX_PAGE_SCOPES).enumerate() {
            pages.push(FiscalIndexPage {
                owner: owner.clone(),
                page: u32::try_from(page).unwrap_or(u32::MAX),
                scopes: chunk.iter().map(|(_, scope)| (*scope).clone()).collect(),
            });
        }
    }
    pages
}

/// The report-queue pages a state derives, without empty ones.
pub(crate) fn queue_pages(state: &FiscalState) -> BTreeMap<String, FiscalReportQueuePage> {
    let mut pages = BTreeMap::<String, FiscalReportQueuePage>::new();
    for (observer, entry) in &state.report_queue {
        pages
            .entry(report_queue_page(observer))
            .or_default()
            .entries
            .insert(observer.clone(), entry.clone());
    }
    pages
}

fn record_of(draft: DomainRecordDraft) -> DomainRecord {
    DomainRecord {
        reference: draft.reference,
        owner: PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    }
}

impl FiscalState {
    /// Partitions an authored state into its records, as a scenario starts
    /// them: the root, the archive head, an office per institution, a ledger
    /// per scope binding, a jurisdiction per region in use, an observer per
    /// observer binding, and the directory buckets, index pages, and
    /// report-queue pages they derive. Derived fields are brought up to date
    /// first (see [`FiscalState::recompute_derived`]), so an author does not
    /// have to; the state must then validate.
    ///
    /// The catalog record is not among them; see
    /// [`CompiledFiscalCatalog::into_record`].
    pub fn into_records(
        mut self,
        catalog: &CompiledFiscalCatalog,
    ) -> Result<Vec<DomainRecord>, CanwuError> {
        let at = self.root.historical_context.updated_at;
        self.recompute_derived(catalog, at);
        self.validate(catalog)?;
        Ok(self.record_drafts()?.into_iter().map(record_of).collect())
    }

    /// The drafts of every record of the partition, without validating.
    pub(crate) fn record_drafts(&self) -> Result<Vec<DomainRecordDraft>, CanwuError> {
        let mut drafts = vec![root_draft(&self.root)?, archive_draft(&self.archive)?];
        for office in self.offices.values() {
            drafts.push(office_draft(office)?);
        }
        for ledger in self.ledgers.values() {
            drafts.push(ledger_draft(ledger)?);
        }
        for jurisdiction in self.jurisdictions.values() {
            drafts.push(jurisdiction_draft(jurisdiction)?);
        }
        for observer in self.observers.values() {
            drafts.push(observer_draft(observer)?);
        }
        for (bucket, entries) in directory_buckets(self) {
            drafts.push(directory_draft(&bucket, &entries)?);
        }
        for page in index_pages(self) {
            drafts.push(index_draft(&page)?);
        }
        for (page, entries) in queue_pages(self) {
            drafts.push(queue_draft(&page, &entries)?);
        }
        Ok(drafts)
    }

    /// Assembles the state from its records: every fiscal record of a run,
    /// the catalog aside, in any order. Records of other kinds are ignored.
    /// The result is not validated; [`audit_fiscal_record_set`] also checks
    /// it and that the records are exactly its partition.
    pub fn from_records<'a>(
        records: impl IntoIterator<Item = &'a DomainRecord>,
    ) -> Result<Self, CanwuError> {
        let mut root = None;
        let mut archive = None;
        let mut state_offices = BTreeMap::new();
        let mut ledgers = BTreeMap::new();
        let mut jurisdictions = BTreeMap::new();
        let mut observers = BTreeMap::new();
        let mut report_queue = BTreeMap::new();
        for record in records {
            if record.lifecycle != DomainRecordLifecycle::Active {
                continue;
            }
            match FiscalRecordKind::of(&record.reference.kind) {
                Some(FiscalRecordKind::Root) => {
                    root = Some(record.decode_payload::<FiscalRootRecord>()?);
                }
                Some(FiscalRecordKind::Archive) => {
                    archive = Some(record.decode_payload::<FiscalArchiveRecord>()?);
                }
                Some(FiscalRecordKind::Office) => {
                    state_offices.insert(
                        record.reference.id.clone(),
                        record.decode_payload::<FiscalOfficeRecord>()?,
                    );
                }
                Some(FiscalRecordKind::Ledger) => {
                    ledgers.insert(
                        record.reference.id.clone(),
                        record.decode_payload::<FiscalLedgerRecord>()?,
                    );
                }
                Some(FiscalRecordKind::Jurisdiction) => {
                    jurisdictions.insert(
                        record.reference.id.clone(),
                        record.decode_payload::<FiscalJurisdictionRecord>()?,
                    );
                }
                Some(FiscalRecordKind::Observer) => {
                    observers.insert(
                        record.reference.id.clone(),
                        record.decode_payload::<FiscalObserverRecord>()?,
                    );
                }
                Some(FiscalRecordKind::ReportQueue) => {
                    let page = record.decode_payload::<FiscalReportQueueRecord>()?;
                    for (observer, entry) in page.entries {
                        if report_queue.insert(observer, entry).is_some() {
                            return Err(invalid("two fiscal report-queue pages hold one observer"));
                        }
                    }
                }
                Some(FiscalRecordKind::Directory | FiscalRecordKind::Index) | None => {}
            }
        }
        Ok(Self {
            root: root.ok_or_else(|| invalid("the fiscal root record is missing"))?,
            archive: archive.ok_or_else(|| invalid("the fiscal archive record is missing"))?,
            offices: state_offices,
            ledgers,
            jurisdictions,
            observers,
            report_queue,
        })
    }
}

/// Checks a set of fiscal records, the catalog among them: the catalog
/// validates, the state they assemble to validates (every record on its own
/// and every invariant across records, derived fields included), and the
/// records are exactly the partition of that state. A directory bucket or
/// report-queue page that is empty may be left over from an earlier
/// partition; every other record must be one the partition holds, with its
/// payload and references. Returns the assembled state.
///
/// It is `O(fiscal records)`. Activation runs it on every scenario and
/// restored run; tests and profiles run it after boundaries.
pub fn audit_fiscal_record_set<'a>(
    records: impl IntoIterator<Item = &'a DomainRecord>,
) -> Result<FiscalState, CanwuError> {
    let records: Vec<&DomainRecord> = records
        .into_iter()
        .filter(|record| {
            record.reference.kind.namespace == FiscalCatalogRecord::NAMESPACE
                && record.lifecycle == DomainRecordLifecycle::Active
        })
        .collect();
    let catalog = records
        .iter()
        .find(|record| record.reference == fiscal_catalog_reference().into_untyped())
        .ok_or_else(|| invalid("the fiscal catalog record is missing"))?
        .decode_payload::<FiscalCatalogRecord>()?;
    catalog.validate()?;
    let state = FiscalState::from_records(records.iter().copied())?;
    state.validate(&catalog)?;
    let mut expected: BTreeMap<DomainRecordRef, DomainRecordDraft> = state
        .record_drafts()?
        .into_iter()
        .map(|mut draft| {
            draft.references.sort();
            (draft.reference.clone(), draft)
        })
        .collect();
    for record in records {
        if record.reference.kind.matches_type::<FiscalCatalogRecord>() {
            continue;
        }
        if FiscalRecordKind::of(&record.reference.kind).is_none() {
            return Err(invalid(format!(
                "{} is not a fiscal record kind",
                record.reference.kind
            )));
        }
        if let Some(draft) = expected.remove(&record.reference) {
            let mut references = record.references.clone();
            references.sort();
            if draft.payload != record.payload || draft.references != references {
                return Err(invalid(format!(
                    "fiscal record {} differs from the partition of the state it belongs to",
                    record.reference
                )));
            }
        } else {
            let empty = match FiscalRecordKind::of(&record.reference.kind) {
                Some(FiscalRecordKind::Directory) => {
                    record.decode_payload::<FiscalDirectoryRecord>()?.is_empty()
                }
                Some(FiscalRecordKind::ReportQueue) => record
                    .decode_payload::<FiscalReportQueueRecord>()?
                    .entries
                    .is_empty(),
                _ => false,
            };
            if !empty {
                return Err(invalid(format!(
                    "fiscal record {} is not part of the state's partition",
                    record.reference
                )));
            }
        }
    }
    if let Some(missing) = expected.keys().next() {
        return Err(invalid(format!("fiscal record {missing} is missing")));
    }
    Ok(state)
}

/// Every fiscal record of a run, the catalog included, in record order.
pub(crate) fn fiscal_records_of<H: CanwuRun + ?Sized>(canwu: &H) -> Vec<&DomainRecord> {
    canwu
        .domain_records()
        .filter(|record| record.reference.kind.namespace == FiscalCatalogRecord::NAMESPACE)
        .collect()
}

/// Checks every fiscal record of a run with [`audit_fiscal_record_set`].
pub fn audit_fiscal_records<H: CanwuRun + ?Sized>(canwu: &H) -> Result<FiscalState, CanwuError> {
    audit_fiscal_record_set(fiscal_records_of(canwu))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        FiscalAssessment, FiscalCycleBook, FiscalMechanism, FiscalOutcomeRing, FiscalPaymentForm,
        FiscalScopeBinding,
    };
    use canwu_api::{
        BoundaryId, DomainRecordVersionRef, OrganizationId, SimTime, SimulationGranularity,
    };

    fn ledger(quotes: usize) -> FiscalScopeLedger {
        let mut book = FiscalCycleBook::open("2026");
        for index in 0..quotes {
            book.assessments.insert(
                format!("assessment.{index}"),
                FiscalAssessment {
                    id: format!("assessment.{index}"),
                    rule_id: "rule".to_owned(),
                    scope_binding_id: "scope".to_owned(),
                    accounting_cycle_id: "2026".to_owned(),
                    quantity: 1,
                    unit: format!("unit.{index}"),
                    payment_form: FiscalPaymentForm::Grain,
                    commutation_quote: Some(DomainRecordVersionRef {
                        record: DomainRecordRef {
                            kind: DomainRecordKind::new("quote.kind", "quote"),
                            id: format!("{index}.{}", "q".repeat(4_000)),
                        },
                        version: 3,
                        established_by: DomainRecordVersionSource::BoundaryChange {
                            boundary: BoundaryId::new(2),
                            change_index: 7,
                        },
                    }),
                    created_at: SimTime::EPOCH,
                    source_action_id: None,
                },
            );
        }
        FiscalScopeLedger {
            scope: FiscalScopeBinding {
                id: "scope".to_owned(),
                institution: EntityRef::Organization(OrganizationId::new(1)),
                jurisdiction_id: "region".to_owned(),
                subject_scope: "land".to_owned(),
                mechanism: FiscalMechanism::LandTax,
                authoritative_granularity: SimulationGranularity::Aggregate,
            },
            office: "office.1".to_owned(),
            rank: 0,
            revision: 1,
            adoptions: BTreeMap::new(),
            books: BTreeMap::from([("2026".to_owned(), book)]),
            watermark: None,
            outcomes: FiscalOutcomeRing::default(),
        }
    }

    /// Settlement's measure of a ledger is the byte length of the payload its
    /// record stores, which activation measures, citations or none.
    #[test]
    fn a_ledger_payload_is_measured_exactly() {
        for quotes in [0, 1, 3] {
            let ledger = ledger(quotes);
            let stored = crate::settle::json_len(&ledger_draft(&ledger).expect("draft").payload)
                .expect("bytes");
            assert_eq!(ledger_payload_bytes(&ledger).expect("bytes"), stored);
        }
    }

    fn hex_suffix(value: &str, prefix: &str) -> usize {
        let digits = value.strip_prefix(prefix).expect("prefix");
        assert!(
            digits
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "{value}"
        );
        usize::from_str_radix(digits, 16).expect("hexadecimal")
    }

    #[test]
    fn buckets_and_pages_are_named_by_three_digits_and_spread() {
        let mut buckets = BTreeSet::new();
        let mut pages = BTreeSet::new();
        for index in 0..20_000 {
            let id = format!("binding.{index}");
            let bucket = directory_bucket(&id);
            assert_eq!(bucket.len(), "dir.".len() + 3);
            assert!(hex_suffix(&bucket, "dir.") < FISCAL_DIRECTORY_BUCKETS);
            assert_eq!(bucket, directory_bucket(&id));
            buckets.insert(bucket);
            let page = report_queue_page(&id);
            assert_eq!(page.len(), "queue.".len() + 3);
            assert!(hex_suffix(&page, "queue.") < FISCAL_REPORT_QUEUE_PAGES);
            pages.insert(page);
        }
        // Twenty thousand IDs reach nearly all of 4,096 buckets.
        assert!(buckets.len() > 3_900, "{}", buckets.len());
        assert!(pages.len() > 3_900, "{}", pages.len());
        // The directory and the queue digest under different domains.
        assert_ne!(
            directory_bucket("binding.0")["dir.".len()..],
            report_queue_page("binding.0")["queue.".len()..]
        );
    }

    #[test]
    fn an_index_page_is_named_by_its_owner_and_number() {
        let office = FiscalIndexOwner::Office("office.1".to_owned());
        let region = FiscalIndexOwner::Jurisdiction("office.1".to_owned());
        assert_ne!(office.page_id(0), region.page_id(0));
        assert_ne!(office.page_id(0), office.page_id(1));
        let (owner, page) = office
            .page_id(7)
            .strip_prefix("index.")
            .and_then(|rest| rest.split_once('.'))
            .map(|(owner, page)| (owner.to_owned(), page.to_owned()))
            .expect("an index page ID");
        assert_eq!(owner.len(), 32);
        assert_eq!(page, "7");
        let per_page = u32::try_from(FISCAL_INDEX_PAGE_SCOPES).expect("scopes");
        assert_eq!(FiscalIndexOwner::locate(0), (0, 0));
        assert_eq!(
            FiscalIndexOwner::locate(per_page - 1),
            (0, FISCAL_INDEX_PAGE_SCOPES - 1)
        );
        assert_eq!(FiscalIndexOwner::locate(per_page), (1, 0));
    }
}
