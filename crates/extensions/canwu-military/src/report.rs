//! Holder-relative engagement reports: the report queue the settle system
//! fills in phase 7 and the publisher drains in phase 13.

use crate::engagement::{Continuation, EngagementOutcomeKind, EngagementStateV1, Side};
use crate::force::LossTallyV1;
use crate::ids::{
    CrossingId, EngagementId, ForceRecord, ForecastBandId, RegistryRecord, RulebookRecord, SiegeId,
    crossing_reference, engagement_reference, engagement_summary_reference, force_reference,
    registry_reference, rulebook_reference, siege_reference,
};
use crate::rulebook::{CombatRulebookV1, EngagementKind};
use crate::siege::{SiegeEndCause, SiegeStatus};
use crate::{CrossingRecord, CrossingStatus, EngagementRecord, PLUGIN_NAMESPACE, SiegeRecord};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryProposal, CanwuError, DomainRecordDraft,
    DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef, EvidenceRef,
    KnowledgeHolderRef, KnowledgeOrigin, KnowledgePublicationGroup, KnowledgeRecordDraft,
    KnowledgeRecordKind, KnowledgeRetention, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeSupersession, PayloadSchema, PluginKnowledgeSchema, SimTime, SimulationView,
    StateVisibility,
};
use serde::{Deserialize, Serialize};

pub const ENGAGEMENT_REPORT_KNOWLEDGE: &str = "engagement_report";
pub const ENGAGEMENT_OUTCOME_KNOWLEDGE: &str = "engagement_outcome";
const SUBJECT_ROLE: &str = "subject";

/// What a report is about.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum ReportSubjectV1 {
    Engagement(EngagementId),
    Siege(SiegeId),
    Crossing(CrossingId),
}

impl ReportSubjectV1 {
    fn record(&self) -> DomainRecordRef {
        match self {
            Self::Engagement(id) => engagement_reference(id).into_untyped(),
            Self::Siege(id) => siege_reference(id).into_untyped(),
            Self::Crossing(id) => crossing_reference(id).into_untyped(),
        }
    }
}

/// One queued report: a subject that stepped or closed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportItemV1 {
    pub subject: ReportSubjectV1,
    pub closing: bool,
    pub boundary_at: SimTime,
}

/// An engagement as one side's report gives it: own figures exact, the
/// enemy's as bands.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngagementPartV1 {
    pub kind: EngagementKind,
    pub forecast_band: ForecastBandId,
    pub own_losses: LossTallyV1,
    /// The band of the enemy's losses, per mille of its men at the opening.
    pub enemy_losses_band: u8,
    /// The bands of the side's latest condition channels.
    pub own_channels: Vec<u8>,
    /// The band of the enemy's men present, per mille of its men at the opening.
    pub enemy_strength_band: u8,
    pub posture: Continuation,
    pub steps: u32,
    pub outcome: Option<EngagementOutcomeKind>,
    pub site_fell: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SiegeReportV1 {
    pub status: SiegeStatus,
    pub end_cause: Option<SiegeEndCause>,
    pub investment_band: u8,
    /// The band of the breach progress, per mille of the threshold.
    pub breach_band: u8,
    pub breach_open: bool,
    /// The defenders' days of stock, in the defenders' report only.
    pub own_days_of_stock: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CrossingReportV1 {
    pub status: CrossingStatus,
    pub waves_sent: u32,
    /// The men landed, in the crossing side's report only.
    pub own_landed: Option<u64>,
    /// The band of the men landed, per mille of the crossing force's men.
    pub landed_band: u8,
    pub bridgehead_secure: bool,
    pub last_wave_intercepted: Option<bool>,
}

/// The body of both report schemas.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngagementReportV1 {
    pub subject: ReportSubjectV1,
    pub side: Side,
    pub closing: bool,
    pub engagement: Option<EngagementPartV1>,
    pub siege: Option<SiegeReportV1>,
    pub crossing: Option<CrossingReportV1>,
    pub as_of: SimTime,
}

/// The progress report schema (each side's next report supersedes its last).
#[must_use]
pub fn engagement_report_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAMESPACE, ENGAGEMENT_REPORT_KNOWLEDGE),
        1,
    )
}

/// The closing report schema (kept).
#[must_use]
pub fn engagement_outcome_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAMESPACE, ENGAGEMENT_OUTCOME_KNOWLEDGE),
        1,
    )
}

pub(crate) fn report_knowledge_schemas() -> Vec<PluginKnowledgeSchema> {
    let subjects = vec![KnowledgeSubjectSchema {
        role: SUBJECT_ROLE.to_owned(),
        targets: vec![
            KnowledgeSubjectTargetKind::Domain(DomainRecordKind::for_type::<EngagementRecord>()),
            KnowledgeSubjectTargetKind::Domain(DomainRecordKind::for_type::<SiegeRecord>()),
            KnowledgeSubjectTargetKind::Domain(DomainRecordKind::for_type::<CrossingRecord>()),
        ],
        required: true,
        multiple: false,
    }];
    let hash = |name: &str| {
        blake3::hash(format!("canwu.military:{name} v1 EngagementReportV1 0.20.0").as_bytes())
            .to_hex()
            .to_string()
    };
    vec![
        PluginKnowledgeSchema {
            id: engagement_report_schema_id(),
            schema_hash: hash(ENGAGEMENT_REPORT_KNOWLEDGE),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: subjects.clone(),
            retention: KnowledgeRetention::RetireWhenSuperseded,
        },
        PluginKnowledgeSchema {
            id: engagement_outcome_schema_id(),
            schema_hash: hash(ENGAGEMENT_OUTCOME_KNOWLEDGE),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects,
            retention: KnowledgeRetention::Keep,
        },
    ]
}

/// Phase 13: drains the report queue in order while the publication budget
/// has room for a whole item, and rewrites the registry with what remains.
pub(crate) fn publish_engagement_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view.typed_domain_record(&registry_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let mut registry = record.decode_payload::<RegistryRecord>()?;
    if registry.report_queue.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let Some(rulebook_id) = registry.rulebook.clone() else {
        return Ok(BoundaryProposal::default());
    };
    let Some(rulebook_record) = view.typed_domain_record(&rulebook_reference(&rulebook_id))? else {
        return Ok(BoundaryProposal::default());
    };
    let rulebook = rulebook_record.decode_payload::<RulebookRecord>()?;
    let mut budget = view.knowledge_publication_budget();
    let queued = registry.report_queue.len();
    let mut directives = Vec::new();
    let mut remaining = Vec::new();
    let mut blocked = false;
    for item in std::mem::take(&mut registry.report_queue) {
        if blocked {
            remaining.push(item);
            continue;
        }
        let Some(groups) = item_groups(view, &rulebook, &item, context.at)? else {
            continue;
        };
        let mut records = 0_usize;
        let mut entries = 0_usize;
        let mut bytes = 0_usize;
        for group in &groups {
            records += group.records.len();
            entries += group.records.len() * group.holders.len();
            for draft in &group.records {
                bytes = bytes.saturating_add(draft.payload_bytes()?);
            }
        }
        if !budget.take(records, entries, bytes) {
            blocked = true;
            remaining.push(item);
            continue;
        }
        directives.push(BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: KnowledgeSupersession::CurrentOnSubjects,
            groups,
            summary: "Publish holder-relative engagement reports".to_owned(),
        });
    }
    if remaining.len() == queued {
        // Nothing fit: the queue waits, unchanged, for a boundary with room.
        return Ok(BoundaryProposal::default());
    }
    registry.report_queue = remaining;
    directives.push(BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Update {
            record: DomainRecordDraft::from_typed(registry_reference(), &registry)?,
            expected_version: record.version,
        },
        summary: "Drain the engagement report queue".to_owned(),
    });
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The publication groups of one item: one per side with an accepted
/// holder. `None` drops the item (its subject is gone or has no holder).
fn item_groups(
    view: &SimulationView<'_>,
    rulebook: &CombatRulebookV1,
    item: &ReportItemV1,
    at: SimTime,
) -> Result<Option<Vec<KnowledgePublicationGroup>>, CanwuError> {
    let subject_record = item.subject.record();
    let Some(record) = view.domain_record(&subject_record)? else {
        return Ok(None);
    };
    if record.lifecycle != DomainRecordLifecycle::Active {
        return Ok(None);
    }
    let mut evidence = Vec::new();
    if let Some(version) = view.current_domain_record_version(&subject_record)? {
        evidence.push(EvidenceRef::DomainRecordVersion(version));
    }
    let (holders, bodies): ([Vec<KnowledgeHolderRef>; 2], [EngagementReportV1; 2]) = match &item
        .subject
    {
        ReportSubjectV1::Engagement(id) => {
            let engagement = record.decode_payload::<EngagementRecord>()?;
            if item.closing
                && let Some(version) = view
                    .current_domain_record_version(engagement_summary_reference(id).as_untyped())?
            {
                evidence.push(EvidenceRef::DomainRecordVersion(version));
            }
            let men = [
                side_men(view, &engagement, Side::Attacker)?,
                side_men(view, &engagement, Side::Defender)?,
            ];
            (
                [
                    engagement.attacker.holders.clone(),
                    engagement.defender.holders.clone(),
                ],
                [Side::Attacker, Side::Defender].map(|side| EngagementReportV1 {
                    subject: item.subject.clone(),
                    side,
                    closing: item.closing,
                    engagement: Some(engagement_part(rulebook, &engagement, side, men)),
                    siege: None,
                    crossing: None,
                    as_of: at,
                }),
            )
        }
        ReportSubjectV1::Siege(_) => {
            let siege = record.decode_payload::<SiegeRecord>()?;
            let breach = u64::from(siege.breach_progress).saturating_mul(1_000)
                / u64::from(siege.breach_threshold.max(1));
            let part = |own_days: Option<u64>| SiegeReportV1 {
                status: siege.status,
                end_cause: siege.end_cause,
                investment_band: rulebook.band(u64::from(siege.investment_bp / 10)),
                breach_band: rulebook.band(breach.min(1_000)),
                breach_open: siege.breach_open,
                own_days_of_stock: own_days,
            };
            (
                siege.holders.clone(),
                [Side::Attacker, Side::Defender].map(|side| EngagementReportV1 {
                    subject: item.subject.clone(),
                    side,
                    closing: item.closing,
                    engagement: None,
                    siege: Some(part(
                        (side == Side::Defender).then_some(siege.supply.days_of_stock),
                    )),
                    crossing: None,
                    as_of: at,
                }),
            )
        }
        ReportSubjectV1::Crossing(_) => {
            let crossing = record.decode_payload::<CrossingRecord>()?;
            let force_men = match view.typed_domain_record(&force_reference(&crossing.force))? {
                Some(force) => force.decode_payload::<ForceRecord>()?.men(),
                None => 0,
            };
            let landed = crossing.landed.saturating_mul(1_000) / force_men.max(1);
            (
                crossing.holders.clone(),
                [Side::Attacker, Side::Defender].map(|side| EngagementReportV1 {
                    subject: item.subject.clone(),
                    side,
                    closing: item.closing,
                    engagement: None,
                    siege: None,
                    crossing: Some(CrossingReportV1 {
                        status: crossing.status,
                        waves_sent: crossing.waves_sent,
                        own_landed: (side == Side::Attacker).then_some(crossing.landed),
                        landed_band: rulebook.band(landed.min(1_000)),
                        bridgehead_secure: crossing.bridgehead_secure,
                        last_wave_intercepted: crossing
                            .last_wave
                            .as_ref()
                            .map(|wave| wave.intercepted),
                    }),
                    as_of: at,
                }),
            )
        }
    };
    let schema = if item.closing {
        engagement_outcome_schema_id()
    } else {
        crate::engagement_report_schema_id()
    };
    let mut groups = Vec::new();
    for (side_holders, body) in holders.into_iter().zip(bodies) {
        let mut accepted = Vec::new();
        for holder in side_holders {
            if view.knowledge_holder_accepts_publication(&holder)? && !accepted.contains(&holder) {
                accepted.push(holder);
            }
        }
        if accepted.is_empty() {
            continue;
        }
        accepted.sort();
        groups.push(KnowledgePublicationGroup {
            holders: accepted,
            records: vec![KnowledgeRecordDraft {
                schema: schema.clone(),
                subjects: vec![KnowledgeSubject {
                    role: SUBJECT_ROLE.to_owned(),
                    target: KnowledgeSubjectTarget::DomainRecord(subject_record.clone()),
                }],
                payload: serde_json::to_value(&body).map_err(|error| {
                    CanwuError::new(canwu_api::ErrorCode::InvalidPayload, error.to_string())
                })?,
                as_of: Some(at),
                confidence_per_mille: 1_000,
                origin: KnowledgeOrigin {
                    method: "canwu.military.engagement-report.v1".to_owned(),
                    evidence: evidence.clone(),
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            }],
        });
    }
    Ok((!groups.is_empty()).then_some(groups))
}

fn side_men(
    view: &SimulationView<'_>,
    engagement: &EngagementStateV1,
    side: Side,
) -> Result<u64, CanwuError> {
    let mut men = 0_u64;
    for force in &engagement.side(side).forces {
        if let Some(record) = view.typed_domain_record(&force_reference(force))? {
            men = men.saturating_add(record.decode_payload::<ForceRecord>()?.men());
        }
    }
    Ok(men)
}

fn engagement_part(
    rulebook: &CombatRulebookV1,
    engagement: &EngagementStateV1,
    side: Side,
    men: [u64; 2],
) -> EngagementPartV1 {
    let own = engagement.side(side);
    let enemy = engagement.side(side.other());
    let opened = enemy.opened_men.max(1);
    EngagementPartV1 {
        kind: engagement.kind,
        forecast_band: engagement.forecast.band.clone(),
        own_losses: own.losses,
        enemy_losses_band: rulebook
            .band((enemy.losses.total().saturating_mul(1_000) / opened).min(1_000)),
        own_channels: own
            .input
            .as_ref()
            .map(|(input, _)| {
                input
                    .channels
                    .iter()
                    .map(|value| rulebook.band(u64::from(*value)))
                    .collect()
            })
            .unwrap_or_default(),
        enemy_strength_band: rulebook
            .band((men[side.other().index()].saturating_mul(1_000) / opened).min(1_000)),
        posture: own.posture_after,
        steps: engagement.steps_resolved,
        outcome: engagement.outcome.map(|outcome| outcome.kind),
        site_fell: engagement.outcome.is_some_and(|outcome| outcome.site_fell),
    }
}
