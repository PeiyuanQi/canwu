//! The fiscal archive: closed accounting cycle books, the per-cycle totals of
//! each institution, and the maintenance receipts of archive commits, kept
//! for good in a store the host application owns.
//!
//! The commit protocol is the resource and production archives': a host
//! prepares a batch from a run's fiscal records ([`crate::prepare_fiscal_archive`]),
//! stores and verifies it ([`PreparedFiscalArchiveBatchV1::store_and_verify`]),
//! queues it ([`crate::enqueue_fiscal_archive`]), and finalizes its retention
//! once it settled ([`crate::finalize_fiscal_archive_retention`]). Lookups
//! go through a cumulative lookup index in the page format of
//! `canwu-resource`, read through the functions that crate makes public; the
//! fiscal store, its errors, and its keys are the fiscal archive's own.
//!
//! No boundary reads the archive store. Settling a commit rebuilds what it
//! binds from the hot state, so a missing, slow, or corrupt store can never
//! fail a boundary, and exact replay of settlement needs no store.

use crate::model::{
    FiscalBookStatus, FiscalCycleBook, FiscalCycleTotal, FiscalScopeBinding, FiscalState,
    fiscal_office_key, invalid, merge_cycle_totals,
};
use canwu_api::{
    CanwuError, DomainRecordVersionSource, EntityRef, ErrorCode, EvidenceCursor, SimTime,
    canonical_hash,
};
use canwu_resource::{ResourceArchiveLookupEntryV1, ResourceArchiveLookupPageV1, ResourceError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

pub const FISCAL_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.fiscal.archive.blob";
pub const FISCAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str = "canwu.fiscal.archive.membership-page";
pub const FISCAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str = "canwu.fiscal.archive.temporal-page";
pub const FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str = "canwu.fiscal.archive.index-directory";
/// Namespace of the pages of the fiscal archive's cumulative lookup index.
/// Its pages have the page format of the resource archive's index; only the
/// namespace and the key digests (see [`fiscal_archive_lookup_key_digest`])
/// are the fiscal archive's own.
pub const FISCAL_ARCHIVE_LOOKUP_PAGE_NAMESPACE: &str = "canwu.fiscal.archive.lookup-page";
/// Format of [`FiscalArchiveBlobV1`]; a blob of another version is refused.
pub const FISCAL_ARCHIVE_BLOB_FORMAT_VERSION: u32 = 1;
/// Records one blob holds, each with its membership and temporal entry.
pub const MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB: usize = 64;
/// Blobs one commit writes.
pub const MAX_FISCAL_ARCHIVE_BLOBS_PER_COMMIT: usize = 64;
/// Records one commit archives: books, the institution totals they complete,
/// and the receipt of the commit before.
pub const MAX_FISCAL_ARCHIVE_COMMIT_RECORDS: usize =
    MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB * MAX_FISCAL_ARCHIVE_BLOBS_PER_COMMIT;
/// Retention handles of settled commits the state holds until the host
/// acknowledges them. Preparation refuses while this many wait
/// ([`FiscalArchiveError::Backpressure`]), which bounds the archive head.
pub const MAX_FISCAL_ARCHIVE_PENDING_HANDLES: usize = 64;

const LOOKUP_KEY_DOMAIN: &str = "canwu.fiscal.archive-lookup-key.v1";
const SOURCE_ROOT_DOMAIN: &str = "canwu.fiscal.archive-source-root.v1";
const BLOB_DOMAIN: &str = "canwu.fiscal.archive-blob.v1";
const BOOK_DOMAIN: &str = "canwu.fiscal.archived-book.v1";
const TOTALS_DOMAIN: &str = "canwu.fiscal.archived-institution-totals.v1";
const RECEIPT_DOMAIN: &str = "canwu.fiscal.archive-maintenance-receipt.v1";
const MEMBERSHIP_DOMAIN: &str = "canwu.fiscal.archive-membership-page.v1";
const TEMPORAL_DOMAIN: &str = "canwu.fiscal.archive-temporal-page.v1";
const DIRECTORY_DOMAIN: &str = "canwu.fiscal.archive-directory.v1";
const RETENTION_DOMAIN: &str = "canwu.fiscal.archive-retention.v1";
const RETENTION_ID_DOMAIN: &str = "canwu.fiscal.archive-retention-id.v1";

/// What went wrong with a fiscal archive operation. None of these ever comes
/// from a boundary: no boundary reads the archive store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FiscalArchiveError {
    /// The store failed to read or write an object or a retention handle.
    Store(String),
    /// A stored object is missing, forged, or malformed.
    Corrupt(String),
    /// The state holds [`MAX_FISCAL_ARCHIVE_PENDING_HANDLES`] retention
    /// handles the host has not acknowledged.
    Backpressure,
    /// The state holds no closed book and no receipt to archive.
    NothingToArchive,
    /// A selected book cites an evidence version a boundary established after
    /// the run's last sealed evidence cut. Seal first, then archive, so that
    /// every citation of an archived book is in a sealed segment once the
    /// book's declaration of it ends.
    UnsealedCitations {
        scope_binding_id: String,
        accounting_cycle_id: String,
    },
    /// A batch or commit is not what the current state derives, or is not
    /// self-consistent.
    Mismatch(String),
    /// The run refused the operation.
    Runtime(CanwuError),
}

impl Display for FiscalArchiveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(message) => write!(formatter, "fiscal archive store failed: {message}"),
            Self::Corrupt(message) => {
                write!(formatter, "fiscal archive object is corrupt: {message}")
            }
            Self::Backpressure => formatter.write_str(
                "archive_backpressure: fiscal archive retention handles wait for acknowledgement",
            ),
            Self::NothingToArchive => formatter.write_str("fiscal archive has nothing to archive"),
            Self::UnsealedCitations {
                scope_binding_id,
                accounting_cycle_id,
            } => write!(
                formatter,
                "fiscal book {accounting_cycle_id} of scope {scope_binding_id} cites evidence newer than the last sealed cut"
            ),
            Self::Mismatch(message) => write!(formatter, "fiscal archive batch differs: {message}"),
            Self::Runtime(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for FiscalArchiveError {}

impl From<CanwuError> for FiscalArchiveError {
    fn from(error: CanwuError) -> Self {
        Self::Runtime(error)
    }
}

impl From<FiscalArchiveError> for CanwuError {
    fn from(error: FiscalArchiveError) -> Self {
        match error {
            FiscalArchiveError::Runtime(error) => error,
            FiscalArchiveError::Backpressure => {
                Self::new(ErrorCode::ValueOutOfRange, error.to_string())
            }
            FiscalArchiveError::NothingToArchive => {
                Self::new(ErrorCode::DomainRecordNotFound, error.to_string())
            }
            other => Self::new(ErrorCode::InvalidArchive, other.to_string()),
        }
    }
}

fn corrupt(message: impl Into<String>) -> FiscalArchiveError {
    FiscalArchiveError::Corrupt(message.into())
}

fn mismatch(message: impl Into<String>) -> FiscalArchiveError {
    FiscalArchiveError::Mismatch(message.into())
}

/// The host application's store of fiscal archive objects.
pub trait FiscalArchiveStore {
    fn store_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), FiscalArchiveError>;

    fn load_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, FiscalArchiveError>;

    fn persist_fiscal_archive_retention(
        &self,
        handle: &FiscalArchiveRetentionHandleV1,
    ) -> Result<(), FiscalArchiveError>;

    fn finalize_fiscal_archive_retention(
        &self,
        handle_id: &str,
        phase: FiscalArchiveRetentionPhaseV1,
    ) -> Result<(), FiscalArchiveError>;
}

/// The absent store: it holds no object and accepts none. It is enough to
/// prepare the first archive commit of a run, which reads nothing, and it
/// fails every read of a state that already has an archive head.
impl FiscalArchiveStore for () {
    fn store_fiscal_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), FiscalArchiveError> {
        Err(FiscalArchiveError::Store(
            "no fiscal archive store is attached".to_owned(),
        ))
    }

    fn load_fiscal_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
    ) -> Result<Option<Vec<u8>>, FiscalArchiveError> {
        Ok(None)
    }

    fn persist_fiscal_archive_retention(
        &self,
        _handle: &FiscalArchiveRetentionHandleV1,
    ) -> Result<(), FiscalArchiveError> {
        Err(FiscalArchiveError::Store(
            "no fiscal archive store is attached".to_owned(),
        ))
    }

    fn finalize_fiscal_archive_retention(
        &self,
        _handle_id: &str,
        _phase: FiscalArchiveRetentionPhaseV1,
    ) -> Result<(), FiscalArchiveError> {
        Err(FiscalArchiveError::Store(
            "no fiscal archive store is attached".to_owned(),
        ))
    }
}

/// The key one archived record is found under.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FiscalArchiveKeyV1 {
    /// The closed book of one accounting cycle of one scope.
    Book {
        scope_binding_id: String,
        accounting_cycle_id: String,
    },
    /// One institution's totals of one accounting cycle, over every book of
    /// the cycle in its scopes.
    InstitutionTotals {
        institution: EntityRef,
        accounting_cycle_id: String,
    },
    /// The maintenance receipt of one applied commit.
    MaintenanceReceipt { receipt_id: String },
}

/// Digest under which the lookup index files one key: a domain-separated
/// BLAKE3 hash, whose hexadecimal nibbles are the key's path through the
/// index.
pub fn fiscal_archive_lookup_key_digest(
    key: &FiscalArchiveKeyV1,
) -> Result<String, FiscalArchiveError> {
    Ok(canonical_hash(LOOKUP_KEY_DOMAIN, key)?)
}

/// One archived book: the closed book with its scope binding and its
/// scope-level totals of the cycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchivedBookV1 {
    pub scope: FiscalScopeBinding,
    pub book: FiscalCycleBook,
    /// One total per (unit, payment form) partition of the book.
    pub totals: Vec<FiscalCycleTotal>,
    pub canonical_digest: String,
}

impl FiscalArchivedBookV1 {
    #[must_use]
    pub fn key(&self) -> FiscalArchiveKeyV1 {
        FiscalArchiveKeyV1::Book {
            scope_binding_id: self.scope.id.clone(),
            accounting_cycle_id: self.book.accounting_cycle_id.clone(),
        }
    }

    fn closed_at(&self) -> Result<SimTime, FiscalArchiveError> {
        match &self.book.status {
            FiscalBookStatus::Closed { closed_by } => Ok(closed_by.rebound_at),
            FiscalBookStatus::Open => Err(corrupt("an archived fiscal book is open")),
        }
    }
}

/// One institution's totals of one accounting cycle: the sum of every book
/// of the cycle in its scopes, archived once, by the commit that archived
/// the last of those books.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchivedInstitutionTotalsV1 {
    pub institution: EntityRef,
    pub accounting_cycle_id: String,
    /// Books the totals sum.
    pub books: u32,
    /// One total per (mechanism, unit, payment form), in that order.
    pub totals: Vec<FiscalCycleTotal>,
    /// When the last of the books closed.
    pub completed_at: SimTime,
    pub canonical_digest: String,
}

impl FiscalArchivedInstitutionTotalsV1 {
    #[must_use]
    pub fn key(&self) -> FiscalArchiveKeyV1 {
        FiscalArchiveKeyV1::InstitutionTotals {
            institution: self.institution.clone(),
            accounting_cycle_id: self.accounting_cycle_id.clone(),
        }
    }
}

/// Where one commit's retention stands, store side and in the state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalArchiveRetentionPhaseV1 {
    Prepared,
    Verified,
    DurableIngress,
    Committed,
    RejectedStale,
    Abandoned,
}

/// What a host keeps to finish or roll back one commit's objects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveRetentionHandleV1 {
    pub handle_id: String,
    pub source_root: String,
    pub target_directory_root: String,
    /// Object IDs by namespace: blobs, membership and temporal pages, and the
    /// directory. Lookup pages are not listed; see
    /// [`PreparedFiscalArchiveBatchV1::superseded_lookup_pages`].
    pub object_ids: BTreeMap<String, BTreeSet<String>>,
    pub phase: FiscalArchiveRetentionPhaseV1,
    pub semantic_digest: String,
}

/// The receipt of one settled commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveMaintenanceReceiptV1 {
    pub id: String,
    pub retention_handle_id: String,
    pub source_root: String,
    pub directory_root: String,
    pub archived_books: u64,
    pub archived_totals: u64,
    pub archived_receipts: u64,
    /// `Committed` for an applied commit, `RejectedStale` for a stale one.
    pub disposition: FiscalArchiveRetentionPhaseV1,
    pub settled_at: SimTime,
    pub canonical_digest: String,
}

impl FiscalArchiveMaintenanceReceiptV1 {
    #[must_use]
    pub fn key(&self) -> FiscalArchiveKeyV1 {
        FiscalArchiveKeyV1::MaintenanceReceipt {
            receipt_id: self.id.clone(),
        }
    }
}

/// The archive head the fiscal state holds.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveHeadV1 {
    /// The directory of the newest applied commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory_root: Option<String>,
    pub archived_book_count: u64,
    pub archived_totals_count: u64,
    pub archived_receipt_count: u64,
    pub committed_batch_count: u64,
    pub receipt_sequence: u64,
    /// The receipt of the applied commit at the head, which the next commit
    /// archives first, and the receipts of stale commits until their handles
    /// are acknowledged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub maintenance_receipts: BTreeMap<String, FiscalArchiveMaintenanceReceiptV1>,
    /// Retention handles of settled commits, until the host acknowledges
    /// them; at most [`MAX_FISCAL_ARCHIVE_PENDING_HANDLES`] applied ones and
    /// as many stale ones.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_handles: BTreeMap<String, FiscalArchiveRetentionHandleV1>,
}

impl FiscalArchiveHeadV1 {
    /// Whether no commit has settled.
    #[must_use]
    pub fn is_unused(&self) -> bool {
        *self == Self::default()
    }

    /// The receipt of the applied commit at the head, if it is hot.
    fn applied_receipts(&self) -> impl Iterator<Item = &FiscalArchiveMaintenanceReceiptV1> {
        self.maintenance_receipts
            .values()
            .filter(|receipt| receipt.disposition == FiscalArchiveRetentionPhaseV1::Committed)
    }

    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        let applied = self.applied_receipts().count();
        let applied_handles = self
            .pending_handles
            .values()
            .filter(|handle| handle.phase == FiscalArchiveRetentionPhaseV1::Committed)
            .count();
        if applied > 1
            || applied_handles > MAX_FISCAL_ARCHIVE_PENDING_HANDLES
            || self.pending_handles.len() > 2 * MAX_FISCAL_ARCHIVE_PENDING_HANDLES
            || self.maintenance_receipts.len() > self.pending_handles.len() + 1
            || self.directory_root.is_some() != (self.committed_batch_count > 0)
            || self.maintenance_receipts.iter().any(|(id, receipt)| {
                *id != receipt.id
                    || !matches!(
                        receipt.disposition,
                        FiscalArchiveRetentionPhaseV1::Committed
                            | FiscalArchiveRetentionPhaseV1::RejectedStale
                    )
            })
            || self.pending_handles.iter().any(|(id, handle)| {
                *id != handle.handle_id
                    || !matches!(
                        handle.phase,
                        FiscalArchiveRetentionPhaseV1::Committed
                            | FiscalArchiveRetentionPhaseV1::RejectedStale
                    )
            })
        {
            return Err(invalid(
                "the fiscal archive head holds more than one applied receipt, too many handles, or entries keyed by another ID",
            ));
        }
        Ok(())
    }
}

/// One content-addressed blob of archived records: receipts, then books,
/// then institution totals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveBlobV1 {
    pub format_version: u32,
    pub expected_source_root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receipts: Vec<FiscalArchiveMaintenanceReceiptV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub books: Vec<FiscalArchivedBookV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub institution_totals: Vec<FiscalArchivedInstitutionTotalsV1>,
    pub content_id: String,
}

impl FiscalArchiveBlobV1 {
    fn len(&self) -> usize {
        self.receipts.len() + self.books.len() + self.institution_totals.len()
    }

    /// Every record as (key, canonical digest, terminal time), in ordinal
    /// order.
    fn entries(&self) -> Result<Vec<(FiscalArchiveKeyV1, &str, SimTime)>, FiscalArchiveError> {
        let mut entries = Vec::with_capacity(self.len());
        for receipt in &self.receipts {
            entries.push((
                receipt.key(),
                receipt.canonical_digest.as_str(),
                receipt.settled_at,
            ));
        }
        for book in &self.books {
            entries.push((
                book.key(),
                book.canonical_digest.as_str(),
                book.closed_at()?,
            ));
        }
        for totals in &self.institution_totals {
            entries.push((
                totals.key(),
                totals.canonical_digest.as_str(),
                totals.completed_at,
            ));
        }
        Ok(entries)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveMembershipV1 {
    pub key: FiscalArchiveKeyV1,
    pub blob_id: String,
    pub ordinal: u16,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveMembershipPageV1 {
    pub id: String,
    pub memberships: Vec<FiscalArchiveMembershipV1>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveTemporalEntryV1 {
    pub terminal_at: SimTime,
    pub key: FiscalArchiveKeyV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveTemporalPageV1 {
    pub id: String,
    pub entries: Vec<FiscalArchiveTemporalEntryV1>,
    pub semantic_digest: String,
}

/// The directory of one commit: its objects, the commit before it, the
/// cumulative counts, and the root of the lookup index over every record
/// archived up to and including it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalArchiveIndexDirectoryV1 {
    pub id: String,
    pub previous_root: Option<String>,
    pub blob_ids: Vec<String>,
    pub membership_pages: Vec<String>,
    pub temporal_pages: Vec<String>,
    pub archived_book_count: u64,
    pub archived_totals_count: u64,
    pub archived_receipt_count: u64,
    pub lookup_root: String,
    pub semantic_digest: String,
}

/// One page of the cumulative lookup index, as a batch stores it. Its format
/// and content ID are the resource archive's index page's.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FiscalArchiveLookupPageV1(ResourceArchiveLookupPageV1);

/// One blob of a batch with its membership and temporal page.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedFiscalArchivePageV1 {
    pub blob: FiscalArchiveBlobV1,
    pub membership_page: FiscalArchiveMembershipPageV1,
    pub temporal_page: FiscalArchiveTemporalPageV1,
}

/// A book a commit selects.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FiscalArchiveBookRefV1 {
    pub scope_binding_id: String,
    pub accounting_cycle_id: String,
}

/// One archive commit, prepared from the fiscal state and the stored index,
/// and not yet stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedFiscalArchiveBatchV1 {
    pub expected_source_root: String,
    /// The receipt of the commit at the head, archived first; at most one.
    pub selected_receipts: Vec<String>,
    pub selected_books: Vec<FiscalArchiveBookRefV1>,
    pub pages: Vec<PreparedFiscalArchivePageV1>,
    /// The lookup pages the commit adds, by ID.
    pub lookup_pages: BTreeMap<String, FiscalArchiveLookupPageV1>,
    /// Pages of the previous index that the new root no longer reaches. A
    /// host that keeps every restore point keeps them; one that only restores
    /// from its latest durable checkpoint may delete them once a checkpoint
    /// taken after this commit is durable, giving up the earlier restore
    /// points.
    pub superseded_lookup_pages: BTreeSet<String>,
    pub directory: FiscalArchiveIndexDirectoryV1,
    pub retention: FiscalArchiveRetentionHandleV1,
}

/// A stored and verified commit, as it is queued.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerifiedFiscalArchiveCommitV1 {
    pub expected_source_root: String,
    pub selected_receipts: Vec<String>,
    pub selected_books: Vec<FiscalArchiveBookRefV1>,
    pub directory_root: String,
    pub lookup_root: String,
    pub retention: FiscalArchiveRetentionHandleV1,
}

/// What settling one archive commit recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FiscalArchiveSettlement {
    /// The commit applied, or settled stale, with this receipt and its
    /// handle recorded in the head.
    Recorded(FiscalArchiveMaintenanceReceiptV1),
    /// A stale copy of a commit whose handle the head already holds: the
    /// same batch queued again. Nothing is recorded.
    Duplicate { handle_id: String },
}

/// One archived record, as [`archived_fiscal_record`] returns it.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum FiscalArchivedRecordV1 {
    Book(FiscalArchivedBookV1),
    InstitutionTotals(FiscalArchivedInstitutionTotalsV1),
    MaintenanceReceipt(FiscalArchiveMaintenanceReceiptV1),
}

/// The fiscal store seen as a store of lookup-index pages, so that the
/// lookup-index functions of `canwu-resource` read and audit the fiscal
/// index. It serves the resource lookup-page namespace from the fiscal one,
/// refuses every other namespace, and writes nothing.
struct FiscalLookupPages<'a>(&'a dyn FiscalArchiveStore);

impl canwu_resource::ResourceArchiveStore for FiscalLookupPages<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the fiscal lookup-page view is read-only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        if namespace != canwu_resource::RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE {
            return Err(ResourceError::InvalidDefinition(format!(
                "the fiscal lookup-page view serves no {namespace} object"
            )));
        }
        self.0
            .load_fiscal_archive_object(FISCAL_ARCHIVE_LOOKUP_PAGE_NAMESPACE, object_id)
            .map_err(|error| ResourceError::InvalidDefinition(error.to_string()))
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &canwu_resource::ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the fiscal lookup-page view keeps no retention".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: canwu_resource::ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Err(ResourceError::InvalidLifecycle(
            "the fiscal lookup-page view keeps no retention".to_owned(),
        ))
    }
}

#[allow(clippy::needless_pass_by_value)]
fn lookup_error(error: ResourceError) -> FiscalArchiveError {
    corrupt(format!("fiscal archive lookup index: {error}"))
}

/// One record a commit archives, in blob order.
enum Candidate<'a> {
    Receipt(&'a FiscalArchiveMaintenanceReceiptV1),
    Book(&'a FiscalScopeBinding, &'a FiscalCycleBook),
    Totals(FiscalArchivedInstitutionTotalsV1),
}

/// The institution totals a selection of books completes: for each
/// (institution, cycle) whose every hot book is selected, the office's
/// archived totals and books plus the selected books'.
fn completed_totals(
    state: &FiscalState,
    books: &[(&FiscalScopeBinding, &FiscalCycleBook)],
) -> Result<Vec<FiscalArchivedInstitutionTotalsV1>, FiscalArchiveError> {
    let mut selected =
        BTreeMap::<(String, String), (&EntityRef, u32, Vec<FiscalCycleTotal>, SimTime)>::new();
    for (scope, book) in books {
        let closed_at = match &book.status {
            FiscalBookStatus::Closed { closed_by } => closed_by.rebound_at,
            FiscalBookStatus::Open => return Err(mismatch("a selected fiscal book is open")),
        };
        let entry = selected
            .entry((
                fiscal_office_key(&scope.institution),
                book.accounting_cycle_id.clone(),
            ))
            .or_insert_with(|| (&scope.institution, 0, Vec::new(), closed_at));
        entry.1 += 1;
        merge_cycle_totals(&mut entry.2, &book.totals(scope.mechanism)?)?;
        entry.3 = entry.3.max(closed_at);
    }
    let mut completed = Vec::new();
    for ((office_key, cycle), (institution, count, totals, completed_at)) in selected {
        let entry = state
            .offices
            .get(&office_key)
            .and_then(|office| office.cycles.get(&cycle))
            .ok_or_else(|| mismatch("a selected fiscal book has no office bookkeeping"))?;
        if entry.hot_books != count {
            continue;
        }
        let mut sum = entry.archived_totals.clone();
        merge_cycle_totals(&mut sum, &totals)?;
        let mut record = FiscalArchivedInstitutionTotalsV1 {
            institution: institution.clone(),
            accounting_cycle_id: cycle,
            books: entry
                .archived_books
                .checked_add(count)
                .ok_or_else(|| mismatch("fiscal archived book count overflowed"))?,
            totals: sum,
            completed_at,
            canonical_digest: String::new(),
        };
        record.canonical_digest = canonical_hash(TOTALS_DOMAIN, &record)?;
        completed.push(record);
    }
    Ok(completed)
}

impl FiscalState {
    /// The digest an archive batch is prepared from and applies against: the
    /// archive head it extends, the revision of every book it selects, and
    /// the cycle bookkeeping of every office cycle those books belong to. A
    /// commit whose source root is not the one the state has now settles as
    /// stale: another commit moved the head or took a book of one of its
    /// cycles, or an audit finding changed a book it selects. Nothing else
    /// makes it stale, so actions, receipts, and context changes on other
    /// books do not.
    pub fn archive_source_root(
        &self,
        books: &[FiscalArchiveBookRefV1],
    ) -> Result<String, CanwuError> {
        let mut selected = Vec::with_capacity(books.len());
        let mut cycles = BTreeMap::<(String, &str), Option<(u32, u32)>>::new();
        for book in books {
            let ledger = self.ledger(&book.scope_binding_id);
            let revision = ledger
                .and_then(|ledger| ledger.books.get(&book.accounting_cycle_id))
                .filter(|hot| !hot.is_open())
                .map(|hot| hot.revision);
            selected.push((&book.scope_binding_id, &book.accounting_cycle_id, revision));
            if let Some(ledger) = ledger {
                let key = (ledger.office.clone(), book.accounting_cycle_id.as_str());
                let entry = self
                    .offices
                    .get(&key.0)
                    .and_then(|office| office.cycles.get(key.1))
                    .map(|entry| (entry.hot_books, entry.archived_books));
                cycles.insert(key, entry);
            }
        }
        canonical_hash(
            SOURCE_ROOT_DOMAIN,
            &(
                &self.archive.directory_root,
                self.archive.archived_book_count,
                self.archive.archived_totals_count,
                self.archive.archived_receipt_count,
                self.archive.committed_batch_count,
                self.archive.receipt_sequence,
                selected,
                cycles.into_iter().collect::<Vec<_>>(),
            ),
        )
    }

    /// The closed books a commit of `book_limit` books would select, in
    /// (scope, cycle) order: as many as fit [`MAX_FISCAL_ARCHIVE_COMMIT_RECORDS`]
    /// together with the head receipt and the institution totals they
    /// complete.
    fn archive_selection(&self, book_limit: usize) -> Vec<(&FiscalScopeBinding, &FiscalCycleBook)> {
        let receipts = self.archive.applied_receipts().count();
        let mut selected: Vec<(&FiscalScopeBinding, &FiscalCycleBook)> = Vec::new();
        let mut per_cycle = BTreeMap::<(String, &str), u32>::new();
        let mut completions = 0_usize;
        for (ledger, book) in self.books() {
            if selected.len() == book_limit {
                break;
            }
            if book.is_open() {
                continue;
            }
            let key = (
                fiscal_office_key(&ledger.scope.institution),
                book.accounting_cycle_id.as_str(),
            );
            let count = per_cycle.get(&key).copied().unwrap_or_default() + 1;
            let hot = self
                .offices
                .get(&key.0)
                .and_then(|office| office.cycles.get(key.1))
                .map_or(0, |entry| entry.hot_books);
            let completes = usize::from(count == hot);
            if receipts + selected.len() + 1 + completions + completes
                > MAX_FISCAL_ARCHIVE_COMMIT_RECORDS
            {
                break;
            }
            per_cycle.insert(key, count);
            completions += completes;
            selected.push((&ledger.scope, book));
        }
        selected
    }

    /// Prepares one archive commit of up to `book_limit` closed books from
    /// this state, which holds at least the archive head, the ledgers of the
    /// first `book_limit` closed books in (scope, cycle) order, and their
    /// offices; see [`crate::prepare_fiscal_archive`], which reads them from a
    /// run, for what the commit holds and when it is refused. `sealed_cut` is
    /// the run's [`canwu_api::CanwuRun::sealed_evidence_cursor`].
    pub(crate) fn prepare_archive_batch(
        &self,
        book_limit: usize,
        store: &dyn FiscalArchiveStore,
        sealed_cut: Option<&EvidenceCursor>,
    ) -> Result<PreparedFiscalArchiveBatchV1, FiscalArchiveError> {
        if book_limit == 0 || book_limit > MAX_FISCAL_ARCHIVE_COMMIT_RECORDS {
            return Err(mismatch("fiscal archive book limit is invalid"));
        }
        if self.archive.pending_handles.len() >= MAX_FISCAL_ARCHIVE_PENDING_HANDLES {
            return Err(FiscalArchiveError::Backpressure);
        }
        let books = self.archive_selection(book_limit);
        let receipts: Vec<_> = self.archive.applied_receipts().collect();
        if books.is_empty() && receipts.is_empty() {
            return Err(FiscalArchiveError::NothingToArchive);
        }
        if let Some(cut) = sealed_cut {
            for (scope, book) in &books {
                if book.citations().any(|version| {
                    matches!(
                        version.established_by,
                        DomainRecordVersionSource::BoundaryChange { boundary, .. }
                            if boundary.get() > cut.boundary_count
                    )
                }) {
                    return Err(FiscalArchiveError::UnsealedCitations {
                        scope_binding_id: scope.id.clone(),
                        accounting_cycle_id: book.accounting_cycle_id.clone(),
                    });
                }
            }
        }
        let selected_books: Vec<FiscalArchiveBookRefV1> = books
            .iter()
            .map(|(scope, book)| FiscalArchiveBookRefV1 {
                scope_binding_id: scope.id.clone(),
                accounting_cycle_id: book.accounting_cycle_id.clone(),
            })
            .collect();
        let expected_source_root = self.archive_source_root(&selected_books)?;
        let pages = self.archive_pages(&expected_source_root, &receipts, &books)?;
        let previous_lookup_root = match &self.archive.directory_root {
            Some(root) => {
                let previous = load_directory(store, root)?;
                if (
                    previous.archived_book_count,
                    previous.archived_totals_count,
                    previous.archived_receipt_count,
                ) != (
                    self.archive.archived_book_count,
                    self.archive.archived_totals_count,
                    self.archive.archived_receipt_count,
                ) {
                    return Err(corrupt(
                        "fiscal archive head directory differs from the head counts",
                    ));
                }
                Some(previous.lookup_root)
            }
            None => None,
        };
        let entries = pages
            .iter()
            .flat_map(|page| &page.membership_page.memberships)
            .map(|membership| {
                Ok(ResourceArchiveLookupEntryV1 {
                    key_digest: fiscal_archive_lookup_key_digest(&membership.key)?,
                    blob_id: membership.blob_id.clone(),
                    ordinal: membership.ordinal,
                })
            })
            .collect::<Result<Vec<_>, FiscalArchiveError>>()?;
        let lookup = canwu_resource::prepare_resource_archive_lookup_entries(
            &FiscalLookupPages(store),
            previous_lookup_root.as_deref(),
            entries,
        )
        .map_err(lookup_error)?;
        let directory = self.archive_directory(&pages, lookup.root)?;
        let mut retention = FiscalArchiveRetentionHandleV1 {
            handle_id: canonical_hash(
                RETENTION_ID_DOMAIN,
                &(&expected_source_root, &directory.id),
            )?,
            source_root: expected_source_root.clone(),
            target_directory_root: directory.id.clone(),
            object_ids: retention_objects(&directory),
            phase: FiscalArchiveRetentionPhaseV1::Prepared,
            semantic_digest: String::new(),
        };
        retention.semantic_digest = canonical_hash(RETENTION_DOMAIN, &retention)?;
        Ok(PreparedFiscalArchiveBatchV1 {
            expected_source_root,
            selected_receipts: receipts.iter().map(|receipt| receipt.id.clone()).collect(),
            selected_books,
            pages,
            lookup_pages: lookup
                .pages
                .into_iter()
                .map(|(id, page)| (id, FiscalArchiveLookupPageV1(page)))
                .collect(),
            superseded_lookup_pages: lookup.superseded,
            directory,
            retention,
        })
    }

    /// The pages of a commit of `receipts` and `books` prepared from
    /// `source_root`: the records in blob order, cut into blobs.
    fn archive_pages(
        &self,
        source_root: &str,
        receipts: &[&FiscalArchiveMaintenanceReceiptV1],
        books: &[(&FiscalScopeBinding, &FiscalCycleBook)],
    ) -> Result<Vec<PreparedFiscalArchivePageV1>, FiscalArchiveError> {
        let candidates: Vec<Candidate<'_>> = receipts
            .iter()
            .map(|receipt| Candidate::Receipt(receipt))
            .chain(
                books
                    .iter()
                    .map(|(scope, book)| Candidate::Book(scope, book)),
            )
            .chain(
                completed_totals(self, books)?
                    .into_iter()
                    .map(Candidate::Totals),
            )
            .collect();
        if candidates.len() > MAX_FISCAL_ARCHIVE_COMMIT_RECORDS {
            return Err(mismatch("fiscal archive commit exceeds its record limit"));
        }
        let mut pages = Vec::new();
        for chunk in candidates.chunks(MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB) {
            let mut blob = FiscalArchiveBlobV1 {
                format_version: FISCAL_ARCHIVE_BLOB_FORMAT_VERSION,
                expected_source_root: source_root.to_owned(),
                receipts: Vec::new(),
                books: Vec::new(),
                institution_totals: Vec::new(),
                content_id: String::new(),
            };
            for candidate in chunk {
                match candidate {
                    Candidate::Receipt(receipt) => blob.receipts.push((*receipt).clone()),
                    Candidate::Book(scope, book) => {
                        let mut record = FiscalArchivedBookV1 {
                            scope: (*scope).clone(),
                            book: (*book).clone(),
                            totals: book.totals(scope.mechanism)?,
                            canonical_digest: String::new(),
                        };
                        record.canonical_digest = canonical_hash(BOOK_DOMAIN, &record)?;
                        blob.books.push(record);
                    }
                    Candidate::Totals(totals) => blob.institution_totals.push(totals.clone()),
                }
            }
            blob.content_id = canonical_hash(BLOB_DOMAIN, &blob)?;
            pages.push(prepare_page(blob)?);
        }
        Ok(pages)
    }

    fn archive_directory(
        &self,
        pages: &[PreparedFiscalArchivePageV1],
        lookup_root: String,
    ) -> Result<FiscalArchiveIndexDirectoryV1, FiscalArchiveError> {
        let add = |total: u64, value: usize| {
            u64::try_from(value)
                .ok()
                .and_then(|value| total.checked_add(value))
                .ok_or_else(|| mismatch("fiscal archive count overflowed"))
        };
        let (books, totals, receipts) = pages.iter().fold((0, 0, 0), |sums, page| {
            (
                sums.0 + page.blob.books.len(),
                sums.1 + page.blob.institution_totals.len(),
                sums.2 + page.blob.receipts.len(),
            )
        });
        let mut directory = FiscalArchiveIndexDirectoryV1 {
            id: String::new(),
            previous_root: self.archive.directory_root.clone(),
            blob_ids: pages
                .iter()
                .map(|page| page.blob.content_id.clone())
                .collect(),
            membership_pages: pages
                .iter()
                .map(|page| page.membership_page.id.clone())
                .collect(),
            temporal_pages: pages
                .iter()
                .map(|page| page.temporal_page.id.clone())
                .collect(),
            archived_book_count: add(self.archive.archived_book_count, books)?,
            archived_totals_count: add(self.archive.archived_totals_count, totals)?,
            archived_receipt_count: add(self.archive.archived_receipt_count, receipts)?,
            lookup_root,
            semantic_digest: String::new(),
        };
        directory.semantic_digest = canonical_hash(DIRECTORY_DOMAIN, &directory)?;
        directory.id.clone_from(&directory.semantic_digest);
        Ok(directory)
    }

    /// Settles one verified commit against this state and records its
    /// maintenance receipt and retention handle.
    ///
    /// A commit prepared from the current state is applied: its books leave
    /// their ledgers, each ledger's watermark rises to the cycle it archived
    /// when that is greater, each office's bookkeeping takes in the books'
    /// totals, and an office whose cycle it emptied raises its watermark to
    /// that cycle. Any other commit is stale and changes nothing else. A
    /// stale copy of a commit whose handle the head already holds, because
    /// the same batch was queued again, records nothing at all; a stale
    /// commit the head has no room for, because it already holds
    /// [`MAX_FISCAL_ARCHIVE_PENDING_HANDLES`] unacknowledged stale handles, is
    /// refused and changes nothing. Neither reads the archive store, and
    /// neither advances any record's revision, so archive maintenance never
    /// makes an action stale.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_fiscal_archive_commit(
        &mut self,
        commit: &VerifiedFiscalArchiveCommitV1,
        at: SimTime,
    ) -> Result<FiscalArchiveSettlement, CanwuError> {
        commit.validate()?;
        let applied =
            self.archive_source_root(&commit.selected_books)? == commit.expected_source_root;
        if !applied {
            if self
                .archive
                .pending_handles
                .contains_key(&commit.retention.handle_id)
            {
                return Ok(FiscalArchiveSettlement::Duplicate {
                    handle_id: commit.retention.handle_id.clone(),
                });
            }
            let stale_handles = self
                .archive
                .pending_handles
                .values()
                .filter(|handle| handle.phase == FiscalArchiveRetentionPhaseV1::RejectedStale)
                .count();
            if stale_handles >= MAX_FISCAL_ARCHIVE_PENDING_HANDLES {
                return Err(invalid(
                    "the fiscal archive head holds as many unacknowledged stale commits as it may",
                ));
            }
        }
        let mut counts = (0_u64, 0_u64, 0_u64);
        if applied {
            let head: Vec<String> = self
                .archive
                .applied_receipts()
                .map(|receipt| receipt.id.clone())
                .collect();
            if commit.selected_receipts != head {
                return Err(invalid(
                    "fiscal archive commit does not archive the receipt of the head commit first",
                ));
            }
            // The blobs, pages, and directory of the selection are rebuilt
            // from this state, with the lookup root the commit names, and
            // must be the ones the commit stored. This reads no store.
            let (pages, completed) = {
                let receipts: Vec<_> = self.archive.applied_receipts().collect();
                let mut books = Vec::with_capacity(commit.selected_books.len());
                for selected in &commit.selected_books {
                    let ledger = self
                        .ledger(&selected.scope_binding_id)
                        .ok_or_else(|| invalid("fiscal archive commit names an unknown scope"))?;
                    let book = ledger
                        .books
                        .get(&selected.accounting_cycle_id)
                        .filter(|book| !book.is_open())
                        .ok_or_else(|| {
                            invalid("fiscal archive commit names a book that is not closed and hot")
                        })?;
                    books.push((&ledger.scope, book));
                }
                (
                    self.archive_pages(&commit.expected_source_root, &receipts, &books)?,
                    completed_totals(self, &books)?,
                )
            };
            let directory = self.archive_directory(&pages, commit.lookup_root.clone())?;
            if directory.id != commit.directory_root
                || commit.retention.object_ids != retention_objects(&directory)
            {
                return Err(invalid(
                    "fiscal archive commit does not bind the exact records of its selection",
                ));
            }
            for receipt in &commit.selected_receipts {
                self.archive.maintenance_receipts.remove(receipt);
            }
            for selected in &commit.selected_books {
                self.archive_hot_book(selected)?;
            }
            for totals in &completed {
                let office = self.office_mut(&totals.institution);
                if office
                    .cycles
                    .get(&totals.accounting_cycle_id)
                    .is_some_and(|entry| entry.hot_books != 0)
                {
                    return Err(invalid(
                        "fiscal archive commit completed a cycle that still has hot books",
                    ));
                }
                office.cycles.remove(&totals.accounting_cycle_id);
                if office
                    .watermark
                    .as_ref()
                    .is_none_or(|watermark| *watermark < totals.accounting_cycle_id)
                {
                    office.watermark = Some(totals.accounting_cycle_id.clone());
                }
            }
            counts = (
                u64::try_from(commit.selected_books.len()).unwrap_or(u64::MAX),
                u64::try_from(completed.len()).unwrap_or(u64::MAX),
                u64::try_from(commit.selected_receipts.len()).unwrap_or(u64::MAX),
            );
            self.archive.directory_root = Some(commit.directory_root.clone());
            self.archive.archived_book_count = self
                .archive
                .archived_book_count
                .checked_add(counts.0)
                .ok_or_else(|| invalid("fiscal archive count overflowed"))?;
            self.archive.archived_totals_count = self
                .archive
                .archived_totals_count
                .checked_add(counts.1)
                .ok_or_else(|| invalid("fiscal archive count overflowed"))?;
            self.archive.archived_receipt_count = self
                .archive
                .archived_receipt_count
                .checked_add(counts.2)
                .ok_or_else(|| invalid("fiscal archive count overflowed"))?;
            self.archive.committed_batch_count = self
                .archive
                .committed_batch_count
                .checked_add(1)
                .ok_or_else(|| invalid("fiscal archive batch count overflowed"))?;
        }
        let disposition = if applied {
            FiscalArchiveRetentionPhaseV1::Committed
        } else {
            FiscalArchiveRetentionPhaseV1::RejectedStale
        };
        self.archive.receipt_sequence = self
            .archive
            .receipt_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("fiscal archive receipt sequence overflowed"))?;
        let mut receipt = FiscalArchiveMaintenanceReceiptV1 {
            id: format!("receipt.{:020}", self.archive.receipt_sequence),
            retention_handle_id: commit.retention.handle_id.clone(),
            source_root: commit.expected_source_root.clone(),
            directory_root: commit.directory_root.clone(),
            archived_books: counts.0,
            archived_totals: counts.1,
            archived_receipts: counts.2,
            disposition,
            settled_at: at,
            canonical_digest: String::new(),
        };
        receipt.canonical_digest = canonical_hash(RECEIPT_DOMAIN, &receipt)?;
        self.archive
            .maintenance_receipts
            .insert(receipt.id.clone(), receipt.clone());
        let mut handle = commit.retention.clone();
        handle.phase = disposition;
        handle.semantic_digest.clear();
        handle.semantic_digest = canonical_hash(RETENTION_DOMAIN, &handle)?;
        self.archive
            .pending_handles
            .insert(handle.handle_id.clone(), handle);
        Ok(FiscalArchiveSettlement::Recorded(receipt))
    }

    /// Takes one closed book out of its ledger and into its office's
    /// archived part of the cycle.
    fn archive_hot_book(&mut self, selected: &FiscalArchiveBookRefV1) -> Result<(), CanwuError> {
        let ledger = self.ledger_mut(&selected.scope_binding_id)?;
        let book = ledger
            .books
            .remove(&selected.accounting_cycle_id)
            .filter(|book| !book.is_open())
            .ok_or_else(|| invalid("fiscal archive commit lost a selected book"))?;
        if ledger
            .watermark
            .as_ref()
            .is_none_or(|watermark| *watermark < selected.accounting_cycle_id)
        {
            ledger.watermark = Some(selected.accounting_cycle_id.clone());
        }
        let institution = ledger.scope.institution.clone();
        let totals = book.totals(ledger.scope.mechanism)?;
        let office = self.office_mut(&institution);
        let entry = office
            .cycles
            .get_mut(&selected.accounting_cycle_id)
            .filter(|entry| entry.hot_books > 0)
            .ok_or_else(|| invalid("fiscal archive commit lost an office's cycle bookkeeping"))?;
        entry.hot_books -= 1;
        entry.archived_books = entry
            .archived_books
            .checked_add(1)
            .ok_or_else(|| invalid("fiscal archived book count overflowed"))?;
        merge_cycle_totals(&mut entry.archived_totals, &totals)
    }

    /// Retires the recovery metadata of one settled commit after the host
    /// finalized its store-side retention: the handle, and every stale
    /// receipt recorded under it. A handle that is already gone, because the
    /// acknowledgement arrived twice, makes this a no-op. The receipt of an
    /// applied commit is not retired here: the next commit archives it.
    pub(crate) fn acknowledge_fiscal_archive_retention(&mut self, handle_id: &str) -> bool {
        if self.archive.pending_handles.remove(handle_id).is_none() {
            return false;
        }
        self.archive.maintenance_receipts.retain(|_, receipt| {
            receipt.disposition != FiscalArchiveRetentionPhaseV1::RejectedStale
                || receipt.retention_handle_id != handle_id
        });
        true
    }
}

fn retention_objects(
    directory: &FiscalArchiveIndexDirectoryV1,
) -> BTreeMap<String, BTreeSet<String>> {
    BTreeMap::from([
        (
            FISCAL_ARCHIVE_BLOB_NAMESPACE.to_owned(),
            directory.blob_ids.iter().cloned().collect(),
        ),
        (
            FISCAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
            directory.membership_pages.iter().cloned().collect(),
        ),
        (
            FISCAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
            directory.temporal_pages.iter().cloned().collect(),
        ),
        (
            FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            BTreeSet::from([directory.id.clone()]),
        ),
    ])
}

/// The membership and temporal pages of one blob.
fn prepare_page(
    blob: FiscalArchiveBlobV1,
) -> Result<PreparedFiscalArchivePageV1, FiscalArchiveError> {
    let entries = blob.entries()?;
    let mut membership_page = FiscalArchiveMembershipPageV1 {
        id: String::new(),
        memberships: entries
            .iter()
            .enumerate()
            .map(|(ordinal, (key, digest, _))| {
                Ok(FiscalArchiveMembershipV1 {
                    key: key.clone(),
                    blob_id: blob.content_id.clone(),
                    ordinal: u16::try_from(ordinal)
                        .map_err(|_| mismatch("fiscal archive ordinal overflowed"))?,
                    semantic_digest: (*digest).to_owned(),
                })
            })
            .collect::<Result<Vec<_>, FiscalArchiveError>>()?,
        semantic_digest: String::new(),
    };
    membership_page.semantic_digest = canonical_hash(MEMBERSHIP_DOMAIN, &membership_page)?;
    membership_page
        .id
        .clone_from(&membership_page.semantic_digest);
    let mut temporal_page = FiscalArchiveTemporalPageV1 {
        id: String::new(),
        entries: entries
            .iter()
            .map(|(key, _, terminal_at)| FiscalArchiveTemporalEntryV1 {
                terminal_at: *terminal_at,
                key: key.clone(),
            })
            .collect(),
        semantic_digest: String::new(),
    };
    temporal_page
        .entries
        .sort_by(|left, right| (left.terminal_at, &left.key).cmp(&(right.terminal_at, &right.key)));
    temporal_page.semantic_digest = canonical_hash(TEMPORAL_DOMAIN, &temporal_page)?;
    temporal_page.id.clone_from(&temporal_page.semantic_digest);
    Ok(PreparedFiscalArchivePageV1 {
        blob,
        membership_page,
        temporal_page,
    })
}

impl PreparedFiscalArchiveBatchV1 {
    /// Records the batch archives.
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.pages.iter().map(|page| page.blob.len()).sum()
    }

    /// Checks that the batch is self-consistent before anything is stored:
    /// page shape and bounds, the selection against the blobs, the
    /// directory's object lists, the lookup page IDs, and the retention
    /// handle. [`Self::store_and_verify`] authenticates the stored commit.
    pub fn validate(&self) -> Result<(), FiscalArchiveError> {
        let mut receipts = Vec::new();
        let mut books = Vec::new();
        let mut pages_bind_their_blobs = true;
        for page in &self.pages {
            let records = page.blob.len();
            receipts.extend(page.blob.receipts.iter().map(|receipt| receipt.id.clone()));
            books.extend(page.blob.books.iter().map(|book| FiscalArchiveBookRefV1 {
                scope_binding_id: book.scope.id.clone(),
                accounting_cycle_id: book.book.accounting_cycle_id.clone(),
            }));
            pages_bind_their_blobs &= records > 0
                && records <= MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB
                && page.blob.expected_source_root == self.expected_source_root
                && page.membership_page.memberships.len() == records
                && page.temporal_page.entries.len() == records;
        }
        if self.pages.is_empty()
            || self.pages.len() > MAX_FISCAL_ARCHIVE_BLOBS_PER_COMMIT
            || self.selected_receipts.len() > 1
            || receipts != self.selected_receipts
            || books != self.selected_books
            || !pages_bind_their_blobs
            || self.directory.previous_root.as_ref() == Some(&self.directory.id)
            || self.directory.blob_ids
                != self
                    .pages
                    .iter()
                    .map(|page| page.blob.content_id.clone())
                    .collect::<Vec<_>>()
            || self.directory.membership_pages
                != self
                    .pages
                    .iter()
                    .map(|page| page.membership_page.id.clone())
                    .collect::<Vec<_>>()
            || self.directory.temporal_pages
                != self
                    .pages
                    .iter()
                    .map(|page| page.temporal_page.id.clone())
                    .collect::<Vec<_>>()
            || !self.lookup_pages.contains_key(&self.directory.lookup_root)
            || self.retention.phase != FiscalArchiveRetentionPhaseV1::Prepared
            || self.retention.source_root != self.expected_source_root
            || self.retention.target_directory_root != self.directory.id
            || self.retention.object_ids != retention_objects(&self.directory)
            || self.retention.handle_id
                != canonical_hash(
                    RETENTION_ID_DOMAIN,
                    &(&self.expected_source_root, &self.directory.id),
                )?
        {
            return Err(mismatch(
                "prepared fiscal archive batch is forged or lacks exact closure",
            ));
        }
        for (id, page) in &self.lookup_pages {
            if &page.0.id().map_err(lookup_error)? != id {
                return Err(mismatch("prepared fiscal archive lookup page is forged"));
            }
        }
        let mut detached = self.retention.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        if digest != canonical_hash(RETENTION_DOMAIN, &detached)? {
            return Err(mismatch(
                "prepared fiscal archive retention handle is forged",
            ));
        }
        Ok(())
    }

    /// Stores every object of the batch, reads each back, and authenticates
    /// the commit: its directory, blobs, records, and pages bind exactly the
    /// selected records, and each resolves through the stored lookup index to
    /// its own blob position.
    pub fn store_and_verify(
        &self,
        store: &dyn FiscalArchiveStore,
    ) -> Result<VerifiedFiscalArchiveCommitV1, FiscalArchiveError> {
        self.validate()?;
        let encode = |value: Result<Vec<u8>, serde_json::Error>| {
            value.map_err(|error| {
                mismatch(format!(
                    "fiscal archive object could not be encoded: {error}"
                ))
            })
        };
        let mut objects = Vec::new();
        for page in &self.pages {
            objects.push((
                FISCAL_ARCHIVE_BLOB_NAMESPACE,
                page.blob.content_id.as_str(),
                encode(serde_json::to_vec(&page.blob))?,
            ));
            objects.push((
                FISCAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
                page.membership_page.id.as_str(),
                encode(serde_json::to_vec(&page.membership_page))?,
            ));
            objects.push((
                FISCAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
                page.temporal_page.id.as_str(),
                encode(serde_json::to_vec(&page.temporal_page))?,
            ));
        }
        for (id, page) in &self.lookup_pages {
            objects.push((
                FISCAL_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
                id.as_str(),
                encode(serde_json::to_vec(page))?,
            ));
        }
        // The directory goes last: once it is stored, everything it names is.
        objects.push((
            FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            self.directory.id.as_str(),
            encode(serde_json::to_vec(&self.directory))?,
        ));
        store.persist_fiscal_archive_retention(&self.retention)?;
        for (namespace, object_id, bytes) in &objects {
            store.store_fiscal_archive_object(namespace, object_id, bytes)?;
            let loaded = store
                .load_fiscal_archive_object(namespace, object_id)?
                .ok_or_else(|| corrupt("fiscal archive readback object is unavailable"))?;
            if &loaded != bytes {
                return Err(corrupt(
                    "fiscal archive readback bytes differ from the stored object",
                ));
            }
        }
        let loaded = load_directory(store, &self.directory.id)?;
        if loaded != self.directory {
            return Err(corrupt("fiscal archive directory readback differs"));
        }
        let closure = authenticate_closure(store, &loaded)?;
        let mut expected = closure
            .records
            .into_iter()
            .map(|(key, blob_id, ordinal)| {
                Ok(ResourceArchiveLookupEntryV1 {
                    key_digest: fiscal_archive_lookup_key_digest(&key)?,
                    blob_id,
                    ordinal,
                })
            })
            .collect::<Result<Vec<_>, FiscalArchiveError>>()?;
        expected.sort();
        canwu_resource::verify_resource_archive_lookup_entries(
            &FiscalLookupPages(store),
            &loaded.lookup_root,
            "",
            &expected,
        )
        .map_err(lookup_error)?;
        let mut retention = self.retention.clone();
        retention.phase = FiscalArchiveRetentionPhaseV1::Verified;
        retention.semantic_digest.clear();
        retention.semantic_digest = canonical_hash(RETENTION_DOMAIN, &retention)?;
        store.persist_fiscal_archive_retention(&retention)?;
        Ok(VerifiedFiscalArchiveCommitV1 {
            expected_source_root: self.expected_source_root.clone(),
            selected_receipts: self.selected_receipts.clone(),
            selected_books: self.selected_books.clone(),
            directory_root: self.directory.id.clone(),
            lookup_root: self.directory.lookup_root.clone(),
            retention,
        })
    }
}

impl VerifiedFiscalArchiveCommitV1 {
    /// Checks that the commit is self-consistent: at most one receipt,
    /// distinct books, at most [`MAX_FISCAL_ARCHIVE_COMMIT_RECORDS`] records,
    /// and a verified retention handle bound to the same source root and
    /// directory. It reads no store.
    pub fn validate(&self) -> Result<(), CanwuError> {
        let mut detached = self.retention.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        let distinct = self.selected_books.iter().collect::<BTreeSet<_>>().len();
        if self.selected_receipts.len() + self.selected_books.len() == 0
            || self.selected_receipts.len() > 1
            || self.selected_books.len() > MAX_FISCAL_ARCHIVE_COMMIT_RECORDS
            || distinct != self.selected_books.len()
            || !is_digest(&self.expected_source_root)
            || !is_digest(&self.directory_root)
            || !is_digest(&self.lookup_root)
            || digest != canonical_hash(RETENTION_DOMAIN, &detached)?
            || !matches!(
                self.retention.phase,
                FiscalArchiveRetentionPhaseV1::Verified
                    | FiscalArchiveRetentionPhaseV1::DurableIngress
            )
            || self.retention.source_root != self.expected_source_root
            || self.retention.target_directory_root != self.directory_root
            || self.retention.handle_id
                != canonical_hash(
                    RETENTION_ID_DOMAIN,
                    &(&self.expected_source_root, &self.directory_root),
                )?
            || self
                .retention
                .object_ids
                .get(FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
                != Some(&BTreeSet::from([self.directory_root.clone()]))
        {
            return Err(invalid(
                "fiscal archive commit is forged or incompletely bound",
            ));
        }
        Ok(())
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn load_encoded<T: DeserializeOwned>(
    store: &dyn FiscalArchiveStore,
    namespace: &str,
    object_id: &str,
) -> Result<Option<T>, FiscalArchiveError> {
    store
        .load_fiscal_archive_object(namespace, object_id)?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|error| {
                corrupt(format!(
                    "fiscal archive object could not be decoded: {error}"
                ))
            })
        })
        .transpose()
}

fn authenticate_directory_identity(
    directory: &FiscalArchiveIndexDirectoryV1,
) -> Result<(), FiscalArchiveError> {
    let mut detached = directory.clone();
    let id = std::mem::take(&mut detached.id);
    let digest = std::mem::take(&mut detached.semantic_digest);
    let expected = canonical_hash(DIRECTORY_DOMAIN, &detached)?;
    if id != expected || digest != expected || !is_digest(&directory.lookup_root) {
        return Err(corrupt("fiscal archive directory is forged"));
    }
    Ok(())
}

/// Loads one directory by ID and authenticates its identity only: the one
/// directory read of a lookup and of a preparation.
fn load_directory(
    store: &dyn FiscalArchiveStore,
    root: &str,
) -> Result<FiscalArchiveIndexDirectoryV1, FiscalArchiveError> {
    let directory: FiscalArchiveIndexDirectoryV1 =
        load_encoded(store, FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, root)?
            .ok_or_else(|| corrupt("fiscal archive directory is unavailable"))?;
    authenticate_directory_identity(&directory)?;
    if directory.id != root {
        return Err(corrupt("fiscal archive directory identity differs"));
    }
    Ok(directory)
}

/// Authenticates one blob read under `blob_id`: its content ID, format,
/// size, and the digest of every record in it.
fn authenticate_blob(blob: &FiscalArchiveBlobV1, blob_id: &str) -> Result<(), FiscalArchiveError> {
    let mut detached = blob.clone();
    detached.content_id.clear();
    if blob.content_id != blob_id
        || blob.content_id != canonical_hash(BLOB_DOMAIN, &detached)?
        || blob.format_version != FISCAL_ARCHIVE_BLOB_FORMAT_VERSION
        || blob.len() == 0
        || blob.len() > MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB
        || !is_digest(&blob.expected_source_root)
    {
        return Err(corrupt("fiscal archive blob content ID is forged"));
    }
    for book in &blob.books {
        let mut detached = book.clone();
        let digest = std::mem::take(&mut detached.canonical_digest);
        if digest != canonical_hash(BOOK_DOMAIN, &detached)?
            || book.book.is_open()
            || book.totals != book.book.totals(book.scope.mechanism)?
        {
            return Err(corrupt("fiscal archived book is forged"));
        }
    }
    for totals in &blob.institution_totals {
        let mut detached = totals.clone();
        let digest = std::mem::take(&mut detached.canonical_digest);
        if digest != canonical_hash(TOTALS_DOMAIN, &detached)? || totals.books == 0 {
            return Err(corrupt("fiscal archived institution totals are forged"));
        }
    }
    for receipt in &blob.receipts {
        let mut detached = receipt.clone();
        let digest = std::mem::take(&mut detached.canonical_digest);
        if digest != canonical_hash(RECEIPT_DOMAIN, &detached)?
            || receipt.disposition != FiscalArchiveRetentionPhaseV1::Committed
        {
            return Err(corrupt("fiscal archive maintenance receipt is forged"));
        }
    }
    Ok(())
}

/// What one authenticated commit holds.
struct CommitClosure {
    /// Every archived key with the blob and ordinal its membership binds.
    records: Vec<(FiscalArchiveKeyV1, String, u16)>,
    books: u64,
    totals: u64,
    receipts: u64,
}

/// Authenticates one commit: the directory's identity, every blob and record
/// it names, and the membership and temporal pages that must cover exactly
/// those records. An archived receipt must be the receipt of the commit
/// before. The lookup index is not read here.
#[allow(clippy::too_many_lines)]
fn authenticate_closure(
    store: &dyn FiscalArchiveStore,
    directory: &FiscalArchiveIndexDirectoryV1,
) -> Result<CommitClosure, FiscalArchiveError> {
    authenticate_directory_identity(directory)?;
    let unique = |values: &[String]| values.iter().collect::<BTreeSet<_>>().len() == values.len();
    if directory.blob_ids.is_empty()
        || directory.blob_ids.len() > MAX_FISCAL_ARCHIVE_BLOBS_PER_COMMIT
        || directory.membership_pages.len() != directory.blob_ids.len()
        || directory.temporal_pages.len() != directory.blob_ids.len()
        || !unique(&directory.blob_ids)
        || !unique(&directory.membership_pages)
        || !unique(&directory.temporal_pages)
        || directory.previous_root.as_ref() == Some(&directory.id)
    {
        return Err(corrupt(
            "fiscal archive directory object lists are empty, duplicated, cyclic, or unbounded",
        ));
    }
    let mut archived = BTreeMap::new();
    let mut source_root: Option<String> = None;
    let (mut books, mut totals, mut receipts) = (0_u64, 0_u64, 0_u64);
    for blob_id in &directory.blob_ids {
        let blob: FiscalArchiveBlobV1 =
            load_encoded(store, FISCAL_ARCHIVE_BLOB_NAMESPACE, blob_id)?
                .ok_or_else(|| corrupt("fiscal archive blob named by the directory is missing"))?;
        authenticate_blob(&blob, blob_id)?;
        if source_root
            .as_ref()
            .is_some_and(|expected| *expected != blob.expected_source_root)
        {
            return Err(corrupt(
                "fiscal archive blobs of one commit differ in their source root",
            ));
        }
        source_root.get_or_insert_with(|| blob.expected_source_root.clone());
        for receipt in &blob.receipts {
            if directory.previous_root.as_ref() != Some(&receipt.directory_root) {
                return Err(corrupt(
                    "a fiscal archived receipt is not the receipt of the commit before",
                ));
            }
        }
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        books = books.saturating_add(count(blob.books.len()));
        totals = totals.saturating_add(count(blob.institution_totals.len()));
        receipts = receipts.saturating_add(count(blob.receipts.len()));
        for (ordinal, (key, digest, terminal_at)) in blob.entries()?.into_iter().enumerate() {
            let ordinal =
                u16::try_from(ordinal).map_err(|_| corrupt("fiscal archive ordinal overflowed"))?;
            if archived
                .insert(
                    key,
                    (blob_id.clone(), ordinal, digest.to_owned(), terminal_at),
                )
                .is_some()
            {
                return Err(corrupt("fiscal archive commit holds a key twice"));
            }
        }
    }
    if receipts > 1 {
        return Err(corrupt(
            "fiscal archive commit archives more than one receipt",
        ));
    }
    let mut membership_keys = BTreeSet::new();
    for page_id in &directory.membership_pages {
        let page: FiscalArchiveMembershipPageV1 =
            load_encoded(store, FISCAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE, page_id)?
                .ok_or_else(|| corrupt("fiscal archive membership page is missing"))?;
        let mut detached = page.clone();
        let id = std::mem::take(&mut detached.id);
        let digest = std::mem::take(&mut detached.semantic_digest);
        let expected = canonical_hash(MEMBERSHIP_DOMAIN, &detached)?;
        if id != *page_id
            || digest != expected
            || id != expected
            || page.memberships.is_empty()
            || page.memberships.len() > MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB
        {
            return Err(corrupt("fiscal archive membership page is forged"));
        }
        for membership in &page.memberships {
            let Some((blob_id, ordinal, digest, _)) = archived.get(&membership.key) else {
                return Err(corrupt("fiscal archive membership names an unknown key"));
            };
            if membership.blob_id != *blob_id
                || membership.ordinal != *ordinal
                || membership.semantic_digest != *digest
                || !membership_keys.insert(membership.key.clone())
            {
                return Err(corrupt(
                    "fiscal archive membership does not bind its exact blob record",
                ));
            }
        }
    }
    let mut temporal_keys = BTreeSet::new();
    for page_id in &directory.temporal_pages {
        let page: FiscalArchiveTemporalPageV1 =
            load_encoded(store, FISCAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE, page_id)?
                .ok_or_else(|| corrupt("fiscal archive temporal page is missing"))?;
        let mut detached = page.clone();
        let id = std::mem::take(&mut detached.id);
        let digest = std::mem::take(&mut detached.semantic_digest);
        let expected = canonical_hash(TEMPORAL_DOMAIN, &detached)?;
        if id != *page_id
            || digest != expected
            || id != expected
            || page.entries.is_empty()
            || page.entries.len() > MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB
            || page.entries.windows(2).any(|pair| {
                (pair[0].terminal_at, &pair[0].key) > (pair[1].terminal_at, &pair[1].key)
            })
        {
            return Err(corrupt(
                "fiscal archive temporal page is forged or unordered",
            ));
        }
        for entry in &page.entries {
            let Some((_, _, _, terminal_at)) = archived.get(&entry.key) else {
                return Err(corrupt("fiscal archive temporal page names an unknown key"));
            };
            if entry.terminal_at != *terminal_at || !temporal_keys.insert(entry.key.clone()) {
                return Err(corrupt(
                    "fiscal archive temporal page does not bind its exact record",
                ));
            }
        }
    }
    if membership_keys.len() != archived.len() || temporal_keys.len() != archived.len() {
        return Err(corrupt(
            "fiscal archive pages do not cover every record of their blobs",
        ));
    }
    Ok(CommitClosure {
        records: archived
            .into_iter()
            .map(|(key, (blob_id, ordinal, _, _))| (key, blob_id, ordinal))
            .collect(),
        books,
        totals,
        receipts,
    })
}

/// Authenticates one commit's directory, blobs, records, and pages; see
/// [`audit_fiscal_archive`] for the whole archive.
pub fn authenticate_fiscal_archive_directory(
    store: &dyn FiscalArchiveStore,
    directory: &FiscalArchiveIndexDirectoryV1,
) -> Result<(), FiscalArchiveError> {
    authenticate_closure(store, directory).map(|_| ())
}

/// Audits the whole fiscal archive of `state`: every commit of the
/// directory chain from the head, the counts the head holds, the lookup index
/// of the head rebuilt from the chain's records (a key archived more than
/// once counts by its newest record), and the objects of every pending
/// retention handle.
///
/// The cost is linear in the archive; it is an offline host step, never part
/// of a boundary or of a restore.
#[allow(clippy::too_many_lines)]
pub fn audit_fiscal_archive(
    store: &dyn FiscalArchiveStore,
    state: &FiscalState,
) -> Result<(), FiscalArchiveError> {
    let head = &state.archive;
    let mut next = head.directory_root.clone();
    let mut expected = (
        head.archived_book_count,
        head.archived_totals_count,
        head.archived_receipt_count,
    );
    let mut visited = BTreeSet::new();
    let mut directories = 0_u64;
    let mut lookup_root = None;
    let mut blob_ids = Vec::new();
    let mut entries = Vec::new();
    while let Some(root) = next {
        if !visited.insert(root.clone()) {
            return Err(corrupt("fiscal archive directory chain is cyclic"));
        }
        directories += 1;
        if directories > head.committed_batch_count {
            return Err(corrupt(
                "fiscal archive directory chain is longer than its committed batches",
            ));
        }
        let directory = load_directory(store, &root)?;
        let closure = authenticate_closure(store, &directory)?;
        if (
            directory.archived_book_count,
            directory.archived_totals_count,
            directory.archived_receipt_count,
        ) != expected
            || closure.receipts != u64::from(directory.previous_root.is_some())
        {
            return Err(corrupt(
                "fiscal archive directory counts or receipts differ from its chain",
            ));
        }
        lookup_root.get_or_insert_with(|| directory.lookup_root.clone());
        let first_blob = blob_ids.len();
        blob_ids.extend(directory.blob_ids.iter().cloned());
        for (key, blob_id, ordinal) in closure.records {
            let blob = directory
                .blob_ids
                .iter()
                .position(|candidate| *candidate == blob_id)
                .and_then(|offset| first_blob.checked_add(offset))
                .and_then(|index| u32::try_from(index).ok())
                .ok_or_else(|| corrupt("fiscal archive blob position overflowed"))?;
            entries.push(
                canwu_resource::ResourceArchiveAuditEntry::new(
                    &fiscal_archive_lookup_key_digest(&key)?,
                    blob,
                    ordinal,
                )
                .map_err(lookup_error)?,
            );
        }
        expected = (
            expected
                .0
                .checked_sub(closure.books)
                .ok_or_else(|| corrupt("fiscal archive book count underflowed"))?,
            expected
                .1
                .checked_sub(closure.totals)
                .ok_or_else(|| corrupt("fiscal archive totals count underflowed"))?,
            expected
                .2
                .checked_sub(closure.receipts)
                .ok_or_else(|| corrupt("fiscal archive receipt count underflowed"))?,
        );
        next = directory.previous_root;
    }
    if expected != (0, 0, 0) || directories != head.committed_batch_count {
        return Err(corrupt(
            "fiscal archive directory chain does not close the head's counts",
        ));
    }
    let rebuilt = canwu_resource::audit_resource_archive_lookup_index(
        &FiscalLookupPages(store),
        &blob_ids,
        entries,
    )
    .map_err(lookup_error)?;
    if rebuilt != lookup_root {
        return Err(corrupt(
            "fiscal archive lookup index differs from its directory chain",
        ));
    }
    for handle in head.pending_handles.values() {
        for (namespace, object_ids) in &handle.object_ids {
            for object_id in object_ids {
                if store
                    .load_fiscal_archive_object(namespace, object_id)?
                    .is_none()
                {
                    return Err(corrupt(
                        "a pending fiscal retention handle names a missing archive object",
                    ));
                }
            }
        }
        let target = load_directory(store, &handle.target_directory_root)?;
        authenticate_closure(store, &target)?;
        if handle.object_ids != retention_objects(&target) {
            return Err(corrupt(
                "a pending fiscal retention handle does not close its exact objects",
            ));
        }
    }
    Ok(())
}

/// Resolves one archived record through the lookup index of the archive
/// head: the head directory, one lookup page per index level, and, for a
/// hit, the one blob that holds the record, each authenticated. A key that
/// was never archived reads only pages. A missing, forged, or malformed
/// object is an error, never a miss.
pub fn archived_fiscal_record(
    state: &FiscalState,
    store: &dyn FiscalArchiveStore,
    key: &FiscalArchiveKeyV1,
) -> Result<Option<FiscalArchivedRecordV1>, FiscalArchiveError> {
    let Some(root) = &state.archive.directory_root else {
        return Ok(None);
    };
    let directory = load_directory(store, root)?;
    let Some(entry) = canwu_resource::find_resource_archive_lookup_entry(
        &FiscalLookupPages(store),
        &directory.lookup_root,
        &fiscal_archive_lookup_key_digest(key)?,
    )
    .map_err(lookup_error)?
    else {
        return Ok(None);
    };
    let blob: FiscalArchiveBlobV1 =
        load_encoded(store, FISCAL_ARCHIVE_BLOB_NAMESPACE, &entry.blob_id)?
            .ok_or_else(|| corrupt("fiscal archive lookup blob is missing"))?;
    authenticate_blob(&blob, &entry.blob_id)?;
    let ordinal = usize::from(entry.ordinal);
    let (receipts, books) = (blob.receipts.len(), blob.books.len());
    let record = if ordinal < receipts {
        blob.receipts
            .into_iter()
            .nth(ordinal)
            .map(FiscalArchivedRecordV1::MaintenanceReceipt)
    } else if ordinal - receipts < books {
        blob.books
            .into_iter()
            .nth(ordinal - receipts)
            .map(FiscalArchivedRecordV1::Book)
    } else {
        blob.institution_totals
            .into_iter()
            .nth(ordinal - receipts - books)
            .map(FiscalArchivedRecordV1::InstitutionTotals)
    }
    .ok_or_else(|| corrupt("fiscal archive lookup ordinal lies outside its blob"))?;
    let found = match &record {
        FiscalArchivedRecordV1::Book(book) => book.key(),
        FiscalArchivedRecordV1::InstitutionTotals(totals) => totals.key(),
        FiscalArchivedRecordV1::MaintenanceReceipt(receipt) => receipt.key(),
    };
    if found != *key {
        return Err(corrupt(
            "fiscal archive lookup entry differs from its record",
        ));
    }
    Ok(Some(record))
}

/// The lookup pages a live run reads: those reachable from the index of the
/// archive head and of each named queued commit. Every page of a named index
/// must be present. A host that lost the superseded pages of a batch uses
/// this to recompute what is live.
pub fn live_fiscal_archive_lookup_pages<'a>(
    state: &FiscalState,
    store: &dyn FiscalArchiveStore,
    queued_directory_roots: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<String>, FiscalArchiveError> {
    let mut pages = BTreeSet::new();
    let mut collect = |root: &str| {
        let directory = load_directory(store, root)?;
        canwu_resource::collect_resource_archive_lookup_pages(
            &FiscalLookupPages(store),
            &directory.lookup_root,
            "",
            true,
            &mut pages,
        )
        .map_err(lookup_error)
    };
    if let Some(root) = &state.archive.directory_root {
        collect(root)?;
    }
    for root in queued_directory_roots {
        collect(root)?;
    }
    Ok(pages)
}

/// Every fiscal archive object reachable from `roots` and from the commits
/// before them, for an offline collection's mark set. The index of each root
/// in `complete_roots` must be whole; any other index is marked as far as the
/// store still holds it.
pub(crate) fn reachable_fiscal_archive_objects(
    store: &dyn FiscalArchiveStore,
    mut roots: BTreeSet<String>,
    complete_roots: &BTreeSet<String>,
    directory_bound: usize,
) -> Result<BTreeMap<&'static str, BTreeSet<String>>, FiscalArchiveError> {
    let mut objects = BTreeMap::<&'static str, BTreeSet<String>>::new();
    let mut lookup_roots = Vec::new();
    let mut visited = BTreeSet::new();
    while let Some(root) = roots.pop_first() {
        if !visited.insert(root.clone()) {
            continue;
        }
        if visited.len() > directory_bound {
            return Err(corrupt(
                "fiscal archive directory graph exceeds its committed bound",
            ));
        }
        let directory = load_directory(store, &root)?;
        lookup_roots.push((
            !complete_roots.contains(&root),
            directory.lookup_root.clone(),
        ));
        objects
            .entry(FISCAL_ARCHIVE_BLOB_NAMESPACE)
            .or_default()
            .extend(directory.blob_ids.iter().cloned());
        objects
            .entry(FISCAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE)
            .or_default()
            .extend(directory.membership_pages.iter().cloned());
        objects
            .entry(FISCAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE)
            .or_default()
            .extend(directory.temporal_pages.iter().cloned());
        objects
            .entry(FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
            .or_default()
            .insert(root);
        if let Some(previous) = directory.previous_root {
            roots.insert(previous);
        }
    }
    // Complete indexes first: a page they share with another index is then
    // already known to be whole.
    lookup_roots.sort();
    let mut lookup_pages = BTreeSet::new();
    for (earlier, lookup_root) in &lookup_roots {
        canwu_resource::collect_resource_archive_lookup_pages(
            &FiscalLookupPages(store),
            lookup_root,
            "",
            !earlier,
            &mut lookup_pages,
        )
        .map_err(lookup_error)?;
    }
    objects.insert(FISCAL_ARCHIVE_LOOKUP_PAGE_NAMESPACE, lookup_pages);
    Ok(objects)
}
