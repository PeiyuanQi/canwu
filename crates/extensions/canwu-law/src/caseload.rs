//! Aggregate caseload: a host reports, per jurisdiction and period, how many
//! cases of each class arrived and were resolved and how many moved through
//! each review level; the law admission system settles each report into one
//! [`AggregateCaseloadRecord`] with its clearance rate and the pending stocks
//! it carries to the next period.
//!
//! The aggregate path is independent of the per-proceeding ledger
//! ([`crate::LegalCase`] and its findings and rulings): it never loads the
//! legal runtime, it keeps its own records (per jurisdiction, one
//! [`LegalCaseloadHead`] and one record per kept period), and those records do
//! not count toward the legal state budget. A host chooses per class of cases which path it uses: classes it
//! reports in aggregate are listed in the plan's
//! [`CaseloadProfileDefinition`]; a case it wants to follow on its own (a
//! party the player knows, an appeal to the capital) it also records as a
//! [`crate::LegalCase`], still counting it in its class's intake, so the
//! aggregate stays the complete count.

use crate::model::{canonical_id, invalid};
use canwu_api::{CanwuError, EvidenceRef, SimTime};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Plugin packet type of a host's aggregate caseload report batch.
pub const LAW_CASELOAD_INGRESS: &str = "caseload_intake_v1";
/// Record kind name of a jurisdiction's caseload head ([`LegalCaseloadHead`]).
pub const LAW_CASELOAD_HEAD: &str = "caseload_head";
/// Record kind name of one settled caseload period
/// ([`AggregateCaseloadRecord`]).
pub const LAW_CASELOAD_PERIOD: &str = "caseload_period";
/// Event the law admission system records for each caseload report it
/// refuses. Its summary names the packet, the report's index in it, the
/// jurisdiction, the period, and a stable reason code
/// ([`CaseloadRefusalReason::code`]); the report changes nothing.
pub const LAW_CASELOAD_REJECTED_EVENT: &str = "canwu.law.caseload_rejected.v1";
/// Wire version of [`LegalCaseloadHead`].
pub const LEGAL_CASELOAD_FORMAT_VERSION: u32 = 1;
/// Most case classes one caseload profile declares.
pub const MAX_CASELOAD_CLASSES: usize = 64;
/// Most review levels one caseload profile declares.
pub const MAX_CASELOAD_REVIEW_LEVELS: usize = 16;
/// Most settled periods a jurisdiction's caseload record keeps.
pub const MAX_CASELOAD_RETAINED_PERIODS: u16 = 120;
/// Largest per-boundary report budget a caseload profile may declare.
pub const MAX_CASELOAD_REPORTS_PER_BOUNDARY: u32 = 16_384;
/// Most evidence references one caseload report cites, after deduplication.
pub const MAX_CASELOAD_EVIDENCE_PER_REPORT: usize = 16;
/// Most bytes one caseload report's evidence encodes to, after deduplication.
pub const MAX_CASELOAD_EVIDENCE_BYTES: usize = 2_048;
/// Ceiling on the payload bytes of all caseload records of a plan. A profile
/// whose worst case exceeds it is refused when the plan compiles
/// ([`CaseloadProfileDefinition::max_state_bytes`]).
pub const MAX_CASELOAD_STATE_BYTES: usize = 256 * 1024 * 1024;

/// Bytes of a period record besides its IDs, counts, and evidence: field
/// names, punctuation, the ordinal, period, clearance, report, and settlement
/// time at their widest.
const PERIOD_RECORD_FIXED_BYTES: usize = 1_024;
/// Bytes of one class in a period record: its ID in three count maps, each
/// with quotes, colon, comma, and a 20-digit count.
const CLASS_ENTRY_BYTES: usize = 26;
/// Bytes of one review level besides its ID: three 20-digit counts with
/// their field names.
const LEVEL_ENTRY_BYTES: usize = 96;
/// Bytes of a head record besides its jurisdiction ID.
const HEAD_RECORD_FIXED_BYTES: usize = 128;

/// The aggregate caseload contract of a legal plan.
///
/// Every list is sorted and deduplicated when the plan compiles. The profile
/// is part of the plan's content hash; a plan without one omits it, so its
/// encoding and hash are those of a plan from before the profile existed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseloadProfileDefinition {
    /// Case classes a host reports in aggregate (canonical IDs, 1 to
    /// [`MAX_CASELOAD_CLASSES`]).
    pub case_classes: Vec<String>,
    /// Review levels whose queues the record keeps (canonical IDs, at most
    /// [`MAX_CASELOAD_REVIEW_LEVELS`]); the order of levels is the host's.
    #[serde(default)]
    pub review_levels: Vec<String>,
    /// Settled periods each jurisdiction keeps, one record each (1 to
    /// [`MAX_CASELOAD_RETAINED_PERIODS`]). A new period overwrites the oldest
    /// kept one, so the cost of settling a report does not grow with it.
    pub retained_periods: u16,
    /// Reports the law admission system settles in one boundary over all
    /// caseload packets (1 to [`MAX_CASELOAD_REPORTS_PER_BOUNDARY`]); a report
    /// past it is refused with [`CaseloadRefusalReason::BoundaryBudget`] and a
    /// packet that carries more on its own is refused whole.
    pub max_reports_per_boundary: u32,
}

impl CaseloadProfileDefinition {
    /// A profile for `case_classes` with no review levels, twelve retained
    /// periods, and 2,048 reports per boundary.
    #[must_use]
    pub fn new(case_classes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            case_classes: case_classes.into_iter().map(Into::into).collect(),
            review_levels: Vec::new(),
            retained_periods: 12,
            max_reports_per_boundary: 2_048,
        }
    }

    pub(crate) fn canonicalize(&mut self) {
        self.case_classes.sort();
        self.case_classes.dedup();
        self.review_levels.sort();
        self.review_levels.dedup();
    }

    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        let mut classes = BTreeSet::new();
        for class in &self.case_classes {
            if !canonical_id(class) || !classes.insert(class) {
                return Err(invalid(format!(
                    "caseload case class {class:?} is not canonical or repeats"
                )));
            }
        }
        let mut levels = BTreeSet::new();
        for level in &self.review_levels {
            if !canonical_id(level) || !levels.insert(level) {
                return Err(invalid(format!(
                    "caseload review level {level:?} is not canonical or repeats"
                )));
            }
        }
        if self.case_classes.is_empty()
            || self.case_classes.len() > MAX_CASELOAD_CLASSES
            || self.review_levels.len() > MAX_CASELOAD_REVIEW_LEVELS
            || self.retained_periods == 0
            || self.retained_periods > MAX_CASELOAD_RETAINED_PERIODS
            || self.max_reports_per_boundary == 0
            || self.max_reports_per_boundary > MAX_CASELOAD_REPORTS_PER_BOUNDARY
        {
            return Err(invalid("caseload profile is outside its limits"));
        }
        Ok(())
    }

    /// The most payload bytes one period record of a jurisdiction whose ID has
    /// `jurisdiction_len` bytes can encode to: a fixed part, the jurisdiction
    /// ID, each class in three count maps, each review level with its three
    /// counts, and [`MAX_CASELOAD_EVIDENCE_BYTES`] of evidence. Every ID is
    /// canonical, so none needs escaping.
    #[must_use]
    pub fn max_period_record_bytes(&self, jurisdiction_len: usize) -> usize {
        let classes = self
            .case_classes
            .iter()
            .map(|class| 3 * (class.len() + CLASS_ENTRY_BYTES))
            .sum::<usize>();
        let levels = self
            .review_levels
            .iter()
            .map(|level| level.len() + LEVEL_ENTRY_BYTES)
            .sum::<usize>();
        PERIOD_RECORD_FIXED_BYTES
            + jurisdiction_len
            + classes
            + levels
            + MAX_CASELOAD_EVIDENCE_BYTES
    }

    /// The most payload bytes of all caseload records of a plan whose
    /// jurisdiction IDs are `jurisdictions`: for each, one head and
    /// `retained_periods` period records,
    /// `sum over jurisdictions (128 + len + retained_periods × max_period_record_bytes(len))`.
    #[must_use]
    pub fn max_state_bytes<'a>(&self, jurisdictions: impl IntoIterator<Item = &'a str>) -> usize {
        jurisdictions
            .into_iter()
            .fold(0_usize, |total, jurisdiction| {
                let len = jurisdiction.len();
                total.saturating_add(
                    (HEAD_RECORD_FIXED_BYTES + len).saturating_add(
                        usize::from(self.retained_periods)
                            .saturating_mul(self.max_period_record_bytes(len)),
                    ),
                )
            })
    }

    fn knows_class(&self, class: &str) -> bool {
        self.case_classes
            .binary_search_by(|known| known.as_str().cmp(class))
            .is_ok()
    }

    fn knows_level(&self, level: &str) -> bool {
        self.review_levels
            .binary_search_by(|known| known.as_str().cmp(level))
            .is_ok()
    }
}

/// A reporting period, `[start, end)`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseloadPeriod {
    pub start: SimTime,
    pub end: SimTime,
}

/// Pending stocks at one instant: first-instance cases awaiting disposition
/// per class, and cases awaiting review per level. Zero entries are omitted.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseloadStock {
    #[serde(default)]
    pub pending_by_class: BTreeMap<String, u64>,
    #[serde(default)]
    pub review_queue_by_level: BTreeMap<String, u64>,
}

/// Cases that entered and left one review level's queue in a period.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseloadReviewFlow {
    pub entered: u64,
    pub concluded: u64,
}

/// One review level of a settled period: its flows and the queue it carries
/// to the next period.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseloadReviewQueue {
    pub entered: u64,
    pub concluded: u64,
    /// Cases awaiting review at this level at the end of the period.
    pub pending: u64,
}

/// One host report of a jurisdiction's caseload in one period, as the host
/// queues it in a [`LAW_CASELOAD_INGRESS`] packet (`{"reports": [..]}`; see
/// [`crate::enqueue_caseload_intake`]).
///
/// Counts are per declared class and level; zero entries may be left out.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseloadIntakeReport {
    /// A jurisdiction the plan compiles.
    pub jurisdiction: String,
    /// The period reported. The first report of a jurisdiction may start at
    /// any time; every later one starts where the jurisdiction's latest
    /// settled period ends. Its end is no later than the boundary that
    /// settles it.
    pub period: CaseloadPeriod,
    /// The stocks the host expects the period to open with. For the
    /// jurisdiction's first report it sets them (absent: zero); afterwards it
    /// must equal the stocks the latest settled period carries (absent: not
    /// checked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<CaseloadStock>,
    #[serde(default)]
    pub intake_by_class: BTreeMap<String, u64>,
    #[serde(default)]
    pub resolved_by_class: BTreeMap<String, u64>,
    #[serde(default)]
    pub review_by_level: BTreeMap<String, CaseloadReviewFlow>,
    /// Evidence the host cites, checked when the report is settled as legal
    /// mutation evidence is (an exact record version must be current, proposed
    /// this boundary, or initial-scenario; an ingress must be queued or
    /// admitted by this boundary). The record keeps the references; it does
    /// not declare them as evidence a seal must keep.
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

/// One settled period of one jurisdiction, persisted as its own record
/// ([`crate::legal_caseload_period_reference`]).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateCaseloadRecord {
    pub jurisdiction: String,
    /// The period's place in the jurisdiction's series, from 1.
    pub ordinal: u64,
    pub period: CaseloadPeriod,
    pub intake_by_class: BTreeMap<String, u64>,
    pub resolved_by_class: BTreeMap<String, u64>,
    /// First-instance cases awaiting disposition at the end of the period:
    /// opening plus intake minus resolved, per class.
    pub pending_by_class: BTreeMap<String, u64>,
    /// Per review level: opening queue plus entered minus concluded.
    pub review_queue_by_level: BTreeMap<String, CaseloadReviewQueue>,
    /// Resolved over intake, all classes together, in basis points (10,000 is
    /// as many resolved as arrived; above it the backlog shrinks), saturating
    /// at `u32::MAX`; absent when nothing arrived.
    pub clearance_bp: Option<u32>,
    /// The host's evidence, sorted and deduplicated.
    pub evidence: Vec<EvidenceRef>,
    /// The packet that carried the report.
    pub report: EvidenceRef,
    /// The boundary time that settled it.
    pub settled_at: SimTime,
}

impl AggregateCaseloadRecord {
    /// The stocks this period carries to the next.
    #[must_use]
    pub fn closing_stock(&self) -> CaseloadStock {
        CaseloadStock {
            pending_by_class: self.pending_by_class.clone(),
            review_queue_by_level: self
                .review_queue_by_level
                .iter()
                .filter(|(_, queue)| queue.pending > 0)
                .map(|(level, queue)| (level.clone(), queue.pending))
                .collect(),
        }
    }

    /// The stocks this period opened with.
    fn opening_stock(&self) -> Option<CaseloadStock> {
        let mut pending_by_class = BTreeMap::new();
        for class in self
            .pending_by_class
            .keys()
            .chain(self.intake_by_class.keys())
            .chain(self.resolved_by_class.keys())
        {
            let count = count(&self.pending_by_class, class)
                .checked_add(count(&self.resolved_by_class, class))?
                .checked_sub(count(&self.intake_by_class, class))?;
            if count > 0 {
                pending_by_class.insert(class.clone(), count);
            }
        }
        let mut review_queue_by_level = BTreeMap::new();
        for (level, queue) in &self.review_queue_by_level {
            let count = queue
                .pending
                .checked_add(queue.concluded)?
                .checked_sub(queue.entered)?;
            if count > 0 {
                review_queue_by_level.insert(level.clone(), count);
            }
        }
        Some(CaseloadStock {
            pending_by_class,
            review_queue_by_level,
        })
    }

    fn review_flows(&self) -> BTreeMap<String, CaseloadReviewFlow> {
        self.review_queue_by_level
            .iter()
            .filter(|(_, queue)| queue.entered > 0 || queue.concluded > 0)
            .map(|(level, queue)| {
                (
                    level.clone(),
                    CaseloadReviewFlow {
                        entered: queue.entered,
                        concluded: queue.concluded,
                    },
                )
            })
            .collect()
    }
}

/// The head record of one jurisdiction's caseload
/// ([`crate::legal_caseload_head_reference`]): how many periods the
/// jurisdiction has settled.
///
/// Each settled period is its own record ([`AggregateCaseloadRecord`],
/// [`crate::legal_caseload_period_reference`]) in one of `retained_periods`
/// slots, the period with ordinal `n` in slot `(n - 1) % retained_periods`, so
/// a new period overwrites the oldest kept one. A jurisdiction therefore
/// holds at most `retained_periods + 1` records, and settling a report writes
/// two small records whatever the retention.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegalCaseloadHead {
    pub format_version: u32,
    pub jurisdiction: String,
    /// Periods settled so far; the newest has this ordinal.
    pub settled_periods: u64,
}

/// One jurisdiction's caseload as [`crate::load_legal_caseload`] reads it:
/// its kept periods, oldest first. The newest period carries the
/// jurisdiction's current pending stocks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalCaseloadState {
    pub jurisdiction: String,
    pub settled_periods: u64,
    pub periods: Vec<AggregateCaseloadRecord>,
}

impl LegalCaseloadState {
    /// The newest settled period.
    #[must_use]
    pub fn latest(&self) -> Option<&AggregateCaseloadRecord> {
        self.periods.last()
    }
}

/// The slot of the period with `ordinal` (from 1).
pub(crate) fn caseload_slot(profile: &CaseloadProfileDefinition, ordinal: u64) -> u64 {
    ordinal.saturating_sub(1) % u64::from(profile.retained_periods.max(1))
}

/// The ordinals a head with `settled` periods keeps, oldest first.
pub(crate) fn kept_ordinals(
    profile: &CaseloadProfileDefinition,
    settled: u64,
) -> std::ops::RangeInclusive<u64> {
    let kept = u64::from(profile.retained_periods.max(1));
    settled.saturating_sub(kept).saturating_add(1)..=settled
}

/// Why the law admission system refused one caseload report.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseloadRefusalReason {
    /// The plan compiles no such jurisdiction.
    JurisdictionUnknown,
    /// A count names a class the profile does not declare.
    ClassUnknown,
    /// A count names a review level the profile does not declare.
    LevelUnknown,
    /// The period is empty or ends after the boundary.
    PeriodInvalid,
    /// The period neither repeats the latest settled period nor starts
    /// where it ends.
    PeriodNotNext,
    /// The report repeats the latest settled period exactly: already
    /// settled, nothing changes.
    PeriodSettled,
    /// The report names the latest settled period with other content.
    PeriodConflict,
    /// The stated opening stocks differ from the stocks the latest settled
    /// period carries.
    OpeningMismatch,
    /// A class would resolve more cases than it had pending plus arrived.
    ResolvedExceedsPending,
    /// A review level would conclude more cases than it had queued plus
    /// entered.
    ReviewExceedsQueue,
    /// A count overflowed.
    CountOverflow,
    /// More evidence than [`MAX_CASELOAD_EVIDENCE_PER_REPORT`] references or
    /// [`MAX_CASELOAD_EVIDENCE_BYTES`] bytes.
    EvidenceBudget,
    /// A cited evidence reference is not available under the rule above.
    EvidenceUnavailable,
    /// The boundary already settled `max_reports_per_boundary` reports.
    BoundaryBudget,
}

impl CaseloadRefusalReason {
    /// The stable reason code in the refusal event's summary.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::JurisdictionUnknown => "caseload_jurisdiction_unknown",
            Self::ClassUnknown => "caseload_class_unknown",
            Self::LevelUnknown => "caseload_level_unknown",
            Self::PeriodInvalid => "caseload_period_invalid",
            Self::PeriodNotNext => "caseload_period_not_next",
            Self::PeriodSettled => "caseload_period_settled",
            Self::PeriodConflict => "caseload_period_conflict",
            Self::OpeningMismatch => "caseload_opening_mismatch",
            Self::ResolvedExceedsPending => "caseload_resolved_exceeds_pending",
            Self::ReviewExceedsQueue => "caseload_review_exceeds_queue",
            Self::CountOverflow => "caseload_count_overflow",
            Self::EvidenceBudget => "caseload_evidence_budget",
            Self::EvidenceUnavailable => "caseload_evidence_unavailable",
            Self::BoundaryBudget => "caseload_boundary_budget",
        }
    }
}

fn count(counts: &BTreeMap<String, u64>, key: &str) -> u64 {
    counts.get(key).copied().unwrap_or(0)
}

fn nonzero(counts: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    counts
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(key, count)| (key.clone(), *count))
        .collect()
}

fn nonzero_flows(
    flows: &BTreeMap<String, CaseloadReviewFlow>,
) -> BTreeMap<String, CaseloadReviewFlow> {
    flows
        .iter()
        .filter(|(_, flow)| flow.entered > 0 || flow.concluded > 0)
        .map(|(level, flow)| (level.clone(), *flow))
        .collect()
}

/// Resolved over intake, all classes together, in basis points; `None` when
/// nothing arrived. Totals are summed in `u128`, so they cannot overflow.
fn clearance_bp(intake: &BTreeMap<String, u64>, resolved: &BTreeMap<String, u64>) -> Option<u32> {
    let total = |counts: &BTreeMap<String, u64>| {
        counts
            .values()
            .map(|count| u128::from(*count))
            .sum::<u128>()
    };
    let intake_total = total(intake);
    (intake_total > 0).then(|| {
        u32::try_from(total(resolved).saturating_mul(10_000) / intake_total).unwrap_or(u32::MAX)
    })
}

fn canonical_stock(stock: &CaseloadStock) -> CaseloadStock {
    CaseloadStock {
        pending_by_class: nonzero(&stock.pending_by_class),
        review_queue_by_level: nonzero(&stock.review_queue_by_level),
    }
}

/// The report's evidence, sorted and deduplicated.
fn canonical_evidence(report: &CaseloadIntakeReport) -> Vec<EvidenceRef> {
    let mut evidence = report.evidence.clone();
    evidence.sort();
    evidence.dedup();
    evidence
}

/// Whether a report names the jurisdiction's newest settled period: an exact
/// repeat is already settled, other content conflicts. Evidence is compared
/// as data, so a retry at a later boundary is decided alike whatever has
/// become of the evidence it cites.
pub(crate) fn repeat_of_newest(
    latest: Option<&AggregateCaseloadRecord>,
    report: &CaseloadIntakeReport,
) -> Option<(CaseloadRefusalReason, String)> {
    let latest = latest.filter(|latest| report.period == latest.period)?;
    let same = nonzero(&report.intake_by_class) == latest.intake_by_class
        && nonzero(&report.resolved_by_class) == latest.resolved_by_class
        && nonzero_flows(&report.review_by_level) == latest.review_flows()
        && canonical_evidence(report) == latest.evidence
        && report
            .opening
            .as_ref()
            .is_none_or(|opening| Some(canonical_stock(opening)) == latest.opening_stock());
    Some(if same {
        (
            CaseloadRefusalReason::PeriodSettled,
            "the period is already settled with this content".to_owned(),
        )
    } else {
        (
            CaseloadRefusalReason::PeriodConflict,
            "the period is already settled with other content".to_owned(),
        )
    })
}

/// Checks the report's evidence against [`MAX_CASELOAD_EVIDENCE_PER_REPORT`]
/// and [`MAX_CASELOAD_EVIDENCE_BYTES`], after deduplication.
pub(crate) fn check_caseload_evidence_budget(
    report: &CaseloadIntakeReport,
) -> Result<Vec<EvidenceRef>, (CaseloadRefusalReason, String)> {
    let evidence = canonical_evidence(report);
    let bytes = serde_json::to_vec(&evidence).map_or(usize::MAX, |encoded| encoded.len());
    if evidence.len() > MAX_CASELOAD_EVIDENCE_PER_REPORT || bytes > MAX_CASELOAD_EVIDENCE_BYTES {
        return Err((
            CaseloadRefusalReason::EvidenceBudget,
            format!(
                "{} references in {bytes} bytes, at most {MAX_CASELOAD_EVIDENCE_PER_REPORT} in \
                 {MAX_CASELOAD_EVIDENCE_BYTES}",
                evidence.len()
            ),
        ));
    }
    Ok(evidence)
}

/// Settles one report against the jurisdiction's newest settled period,
/// after the plugin checked the jurisdiction and the evidence. Returns the
/// new period, or why the report is refused; a refusal changes nothing.
pub(crate) fn settle_caseload_report(
    profile: &CaseloadProfileDefinition,
    latest: Option<&AggregateCaseloadRecord>,
    report: &CaseloadIntakeReport,
    report_ref: EvidenceRef,
    at: SimTime,
) -> Result<AggregateCaseloadRecord, (CaseloadRefusalReason, String)> {
    use CaseloadRefusalReason as Reason;
    if let Some(repeat) = repeat_of_newest(latest, report) {
        return Err(repeat);
    }
    if report.period.start >= report.period.end || report.period.end > at {
        return Err((
            Reason::PeriodInvalid,
            "a period must be non-empty and end no later than the boundary".to_owned(),
        ));
    }
    let opening_classes = report
        .opening
        .iter()
        .flat_map(|stock| stock.pending_by_class.keys());
    if let Some(class) = report
        .intake_by_class
        .keys()
        .chain(report.resolved_by_class.keys())
        .chain(opening_classes)
        .find(|class| !profile.knows_class(class))
    {
        return Err((Reason::ClassUnknown, format!("class {class}")));
    }
    let opening_levels = report
        .opening
        .iter()
        .flat_map(|stock| stock.review_queue_by_level.keys());
    if let Some(level) = report
        .review_by_level
        .keys()
        .chain(opening_levels)
        .find(|level| !profile.knows_level(level))
    {
        return Err((Reason::LevelUnknown, format!("level {level}")));
    }
    let evidence = check_caseload_evidence_budget(report)?;
    let intake = nonzero(&report.intake_by_class);
    let resolved = nonzero(&report.resolved_by_class);
    let flows = nonzero_flows(&report.review_by_level);
    if let Some(latest) = latest {
        if report.period.start != latest.period.end {
            return Err((
                Reason::PeriodNotNext,
                format!(
                    "the next period starts at minute {}",
                    latest.period.end.as_minutes()
                ),
            ));
        }
    }
    let opening = match latest {
        Some(latest) => {
            let carried = latest.closing_stock();
            if report
                .opening
                .as_ref()
                .is_some_and(|opening| canonical_stock(opening) != carried)
            {
                return Err((
                    Reason::OpeningMismatch,
                    "the opening stocks differ from the stocks the latest period carries"
                        .to_owned(),
                ));
            }
            carried
        }
        None => report
            .opening
            .as_ref()
            .map(canonical_stock)
            .unwrap_or_default(),
    };
    let overflow = || (Reason::CountOverflow, "a count overflowed".to_owned());
    let mut pending_by_class = BTreeMap::new();
    for class in &profile.case_classes {
        let available = count(&opening.pending_by_class, class)
            .checked_add(count(&intake, class))
            .ok_or_else(overflow)?;
        let pending = available
            .checked_sub(count(&resolved, class))
            .ok_or_else(|| {
                (
                    Reason::ResolvedExceedsPending,
                    format!("class {class} resolves more than it had pending"),
                )
            })?;
        if pending > 0 {
            pending_by_class.insert(class.clone(), pending);
        }
    }
    let mut review_queue_by_level = BTreeMap::new();
    for level in &profile.review_levels {
        let flow = flows.get(level).copied().unwrap_or_default();
        let available = count(&opening.review_queue_by_level, level)
            .checked_add(flow.entered)
            .ok_or_else(overflow)?;
        let pending = available.checked_sub(flow.concluded).ok_or_else(|| {
            (
                Reason::ReviewExceedsQueue,
                format!("level {level} concludes more than it had queued"),
            )
        })?;
        if pending > 0 || flow.entered > 0 || flow.concluded > 0 {
            review_queue_by_level.insert(
                level.clone(),
                CaseloadReviewQueue {
                    entered: flow.entered,
                    concluded: flow.concluded,
                    pending,
                },
            );
        }
    }
    let ordinal = latest
        .map_or(Some(1), |latest| latest.ordinal.checked_add(1))
        .ok_or_else(overflow)?;
    let clearance_bp = clearance_bp(&intake, &resolved);
    Ok(AggregateCaseloadRecord {
        jurisdiction: report.jurisdiction.clone(),
        ordinal,
        period: report.period,
        intake_by_class: intake,
        resolved_by_class: resolved,
        pending_by_class,
        review_queue_by_level,
        clearance_bp,
        evidence,
        report: report_ref,
        settled_at: at,
    })
}

/// Checks one jurisdiction's persisted caseload (its head and the period
/// records it keeps, by slot) against the plan's profile and jurisdictions
/// when the law plugin activates.
pub(crate) fn validate_caseload_records(
    profile: Option<&CaseloadProfileDefinition>,
    jurisdiction_known: impl Fn(&str) -> bool,
    head_id: &str,
    head: &LegalCaseloadHead,
    periods: &BTreeMap<u64, (String, AggregateCaseloadRecord)>,
) -> Result<(), CanwuError> {
    let profile =
        profile.ok_or_else(|| invalid("a caseload record exists but the plan has no profile"))?;
    if head.format_version != LEGAL_CASELOAD_FORMAT_VERSION
        || !jurisdiction_known(&head.jurisdiction)
        || head_id != crate::caseload_head_record_id(&head.jurisdiction)
        || head.settled_periods == 0
    {
        return Err(invalid(format!(
            "caseload head {head_id} is not a valid head of its jurisdiction"
        )));
    }
    let ordinals = kept_ordinals(profile, head.settled_periods);
    if periods.len() as u64 != ordinals.end() - ordinals.start() + 1 {
        return Err(invalid(format!(
            "caseload head {head_id} does not keep exactly its retained periods"
        )));
    }
    let mut previous: Option<&AggregateCaseloadRecord> = None;
    for ordinal in ordinals {
        let slot = caseload_slot(profile, ordinal);
        let Some((record_id, period)) = periods.get(&slot) else {
            return Err(invalid(format!(
                "caseload head {head_id} lacks the period in slot {slot}"
            )));
        };
        let canonical = *record_id == crate::caseload_period_record_id(&head.jurisdiction, slot)
            && period.jurisdiction == head.jurisdiction
            && period.ordinal == ordinal
            && period.period.start < period.period.end
            && period.settled_at >= period.period.end
            && period
                .intake_by_class
                .iter()
                .chain(&period.resolved_by_class)
                .chain(&period.pending_by_class)
                .all(|(class, count)| *count > 0 && profile.knows_class(class))
            && period.review_queue_by_level.iter().all(|(level, queue)| {
                profile.knows_level(level)
                    && (queue.entered > 0 || queue.concluded > 0 || queue.pending > 0)
            })
            && period.evidence.windows(2).all(|pair| pair[0] < pair[1])
            && period.evidence.len() <= MAX_CASELOAD_EVIDENCE_PER_REPORT
            && serde_json::to_vec(&period.evidence)
                .is_ok_and(|encoded| encoded.len() <= MAX_CASELOAD_EVIDENCE_BYTES)
            && period.clearance_bp
                == clearance_bp(&period.intake_by_class, &period.resolved_by_class)
            && period.opening_stock().is_some()
            && previous.is_none_or(|previous| {
                previous.period.end == period.period.start
                    && previous.settled_at <= period.settled_at
                    && Some(previous.closing_stock()) == period.opening_stock()
            });
        if !canonical {
            return Err(invalid(format!(
                "caseload period {record_id} is not canonical for its head"
            )));
        }
        previous = Some(period);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{
        BoundaryId, DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource, IngressId,
    };

    /// The bound a plan is compiled against holds for the widest record a
    /// report can settle: the longest IDs, every count at its widest, and
    /// evidence up to its byte cap.
    #[test]
    fn the_widest_caseload_records_fit_their_bound() {
        let id = |prefix: &str, index: usize| {
            format!("{prefix}{index:0>width$}", width = 128 - prefix.len())
        };
        let profile = CaseloadProfileDefinition {
            case_classes: (0..MAX_CASELOAD_CLASSES)
                .map(|index| id("c", index))
                .collect(),
            review_levels: (0..MAX_CASELOAD_REVIEW_LEVELS)
                .map(|index| id("l", index))
                .collect(),
            retained_periods: MAX_CASELOAD_RETAINED_PERIODS,
            max_reports_per_boundary: MAX_CASELOAD_REPORTS_PER_BOUNDARY,
        };
        profile.validate().expect("the widest profile is valid");
        let jurisdiction = id("j", 0);
        let mut evidence = Vec::new();
        for index in 0..MAX_CASELOAD_EVIDENCE_PER_REPORT {
            let mut candidate = evidence.clone();
            candidate.push(EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
                record: DomainRecordRef::new("h", "r", id("r", index)),
                version: u64::MAX,
                established_by: DomainRecordVersionSource::BoundaryChange {
                    boundary: BoundaryId::new(u64::MAX),
                    change_index: u64::MAX,
                },
            }));
            if serde_json::to_vec(&candidate)
                .expect("evidence encodes")
                .len()
                > MAX_CASELOAD_EVIDENCE_BYTES
            {
                break;
            }
            evidence = candidate;
        }
        evidence.sort();
        let widest = |keys: &[String]| keys.iter().map(|key| (key.clone(), u64::MAX)).collect();
        let record = AggregateCaseloadRecord {
            jurisdiction: jurisdiction.clone(),
            ordinal: u64::MAX,
            period: CaseloadPeriod {
                start: SimTime::from_minutes(i64::MIN),
                end: SimTime::from_minutes(i64::MAX),
            },
            intake_by_class: widest(&profile.case_classes),
            resolved_by_class: widest(&profile.case_classes),
            pending_by_class: widest(&profile.case_classes),
            review_queue_by_level: profile
                .review_levels
                .iter()
                .map(|level| {
                    (
                        level.clone(),
                        CaseloadReviewQueue {
                            entered: u64::MAX,
                            concluded: u64::MAX,
                            pending: u64::MAX,
                        },
                    )
                })
                .collect(),
            clearance_bp: Some(u32::MAX),
            evidence,
            report: EvidenceRef::Ingress(IngressId::new(u64::MAX)),
            settled_at: SimTime::from_minutes(i64::MIN),
        };
        let bytes = serde_json::to_vec(&record).expect("record encodes").len();
        assert!(
            bytes <= profile.max_period_record_bytes(jurisdiction.len()),
            "{bytes}"
        );
        let head = LegalCaseloadHead {
            format_version: u32::MAX,
            jurisdiction: jurisdiction.clone(),
            settled_periods: u64::MAX,
        };
        let head_bytes = serde_json::to_vec(&head).expect("head encodes").len();
        assert!(head_bytes <= HEAD_RECORD_FIXED_BYTES + jurisdiction.len());
    }
}
