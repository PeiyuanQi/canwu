#[path = "../peak_rss.rs"]
mod peak_rss;

use canwu_api::{
    BoundaryRequest, Canwu, Command, CommandEnvelope, CommandRequest, CommandRequestId,
    DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordRef, DomainRecordType,
    DomainReference, DomainReferenceTarget, EntityRef, Government, GovernmentId, Issuer,
    KnowledgeHolderRef, KnowledgeSnapshot, MapPoint, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadRequiredEvidenceContinuationV1, Person, PersonId, ReplayJournal, Scenario, SimDuration,
    SimTime, SimulationPlugin, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_history_research::{
    ASSESSMENT_COMMAND, AssessmentCore, AssessmentRecord, HistoricalAssessmentCommand,
    HistoricalPracticeAssessment, HistoricalPracticeAssessmentPayload, HistoricalResearchSuite,
    HistoricalSourcesAssessment, HistoricalSourcesAssessmentPayload,
    ProductionArchaeologyAssessment, ProductionArchaeologyAssessmentPayload,
    validate_historical_research_runtime,
};
use canwu_technology::{
    ApplicationSpecPayload, MetricComparison, MetricSchema, MetricSchemaPayload, MetricThreshold,
    ProgramMode, ProgramStatus, REFERENCE_EVALUATOR_V1, RequirementGroup, TECHNOLOGY_COMMAND,
    TechnicalClaimPayload, TechnicalProgram, TechnicalProgramPayload, TechniqueRevision,
    TechniqueRevisionPayload, TechniqueSpec, TechniqueSpecPayload, TechnologyCatalogRecord,
    TechnologyCommandEnvelope, TechnologyPlugin, TechnologyRecordChange, TechnologyRecordPayload,
    TransmissionMode, TransmissionOpportunity, TransmissionOpportunityPayload,
    initial_record_version, validate_technology_runtime,
};
use serde_json::{Value, json};
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Profile {
    Interactive,
    Pressure,
}

impl Profile {
    const fn label(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Pressure => "pressure",
        }
    }

    const fn dimensions(self) -> Dimensions {
        match self {
            Self::Interactive => Dimensions {
                sites: 100,
                programs: 200,
                transmissions: 400,
                assessments_per_plugin: 48,
            },
            Self::Pressure => Dimensions {
                sites: 500,
                programs: 1_000,
                transmissions: 2_000,
                assessments_per_plugin: 256,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct Dimensions {
    sites: usize,
    programs: usize,
    transmissions: usize,
    assessments_per_plugin: usize,
}

struct Options {
    profile: Profile,
    samples: usize,
    warmup: usize,
    months: usize,
    machine: String,
    recorded_on: String,
    output: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = options()?;
    let dimensions = options.profile.dimensions();
    eprintln!(
        "building {} technology profile: {} sites, {} programs, {} links",
        options.profile.label(),
        dimensions.sites,
        dimensions.programs,
        dimensions.transmissions
    );

    let build_started = Instant::now();
    let (scenario, revision) = fixture(dimensions)?;
    let technology = TechnologyPlugin;
    let history = HistoricalResearchSuite::plugins();
    let plugins: [&dyn SimulationPlugin; 4] = [&technology, history[0], history[1], history[2]];
    let canwu = Canwu::new_with_plugins(0xCA_4E_55, scenario, &plugins)?;
    validate_technology_runtime(&canwu)?;
    validate_historical_research_runtime(&canwu)?;
    let initial_counts = record_counts(&canwu)?;
    let build_ms = milliseconds(build_started.elapsed());

    for _ in 0..options.warmup {
        measure_operation(&canwu, &revision)?;
    }
    let mut operation_ms = Vec::with_capacity(options.samples);
    for _ in 0..options.samples {
        operation_ms.push(measure_operation(&canwu, &revision)?);
    }

    let initial_snapshot_bytes = canwu.snapshot_json()?.len();
    let mut monthly = canwu.fork();
    let mut monthly_ms = Vec::with_capacity(options.months);
    for month in 0..options.months {
        let at = monthly
            .time()
            .checked_add(SimDuration::days(30))
            .ok_or("monthly benchmark time overflow")?;
        let started = Instant::now();
        apply_monthly_workload(&mut monthly, &revision, month, at)?;
        monthly_ms.push(milliseconds(started.elapsed()));
    }
    let final_counts = record_counts(&monthly)?;
    final_counts.assert_monthly_growth(initial_counts, options.months)?;

    let snapshot_started = Instant::now();
    let snapshot = monthly.snapshot_json()?;
    let snapshot_ms = milliseconds(snapshot_started.elapsed());
    let checkpoint = serde_json::to_vec(&monthly.checkpoint()?)?;
    let checkpoint_journal = monthly.checkpoint_journal_json()?;
    let replay_journal = monthly.replay_journal();
    let replay_json = serde_json::to_vec(&replay_journal)?;

    let persistence = measure_file_persistence(
        options.profile,
        snapshot.as_bytes(),
        checkpoint_journal.as_bytes(),
        &replay_json,
    )?;

    let load_started = Instant::now();
    let restored =
        canwu_technology::from_technology_snapshot_json(&persistence.snapshot, &plugins)?;
    validate_historical_research_runtime(&restored)?;
    let load_ms = milliseconds(load_started.elapsed());

    let checkpoint_load_started = Instant::now();
    let checkpoint_restored = Canwu::from_checkpoint_journal_json_with_plugins(
        &persistence.checkpoint_journal,
        &plugins,
    )?;
    validate_technology_runtime(&checkpoint_restored)?;
    validate_historical_research_runtime(&checkpoint_restored)?;
    let checkpoint_load_ms = milliseconds(checkpoint_load_started.elapsed());

    let replay_started = Instant::now();
    let replayed =
        canwu_technology::replay_technology_from_journal(&plugins, &persistence.replay_journal)?;
    validate_historical_research_runtime(&replayed)?;
    let replay_ms = milliseconds(replay_started.elapsed());
    if replayed.snapshot() != monthly.snapshot() {
        return Err("technology monthly replay did not reproduce the authoritative state".into());
    }

    let snapshot_growth = snapshot.len().saturating_sub(initial_snapshot_bytes);
    let annual_snapshot_growth = snapshot_growth
        .saturating_mul(12)
        .checked_div(options.months)
        .unwrap_or_default();

    let report = json!({
        "benchmark": "canwu-technology-home-hardware-v1",
        "profile": options.profile.label(),
        "machine": options.machine,
        "recorded_on": options.recorded_on,
        "source": source_identity(),
        "dimensions": {
            "sites": dimensions.sites,
            "programs": dimensions.programs,
            "transmission_opportunities": dimensions.transmissions,
            "assessments_per_plugin": dimensions.assessments_per_plugin,
            "history_plugins": 3,
            "months": options.months,
        },
        "final_record_counts": {
            "technology": final_counts.technology,
            "technology_cap": canwu_technology::TechnologyLimitsV1::canonical().max_total_records,
            "technology_cap_utilization_per_mille": final_counts.technology * 1_000
                / canwu_technology::TechnologyLimitsV1::canonical().max_total_records,
            "knowledge": final_counts.knowledge,
            "knowledge_cap": canwu_technology::TechnologyLimitsV1::canonical().max_knowledge_records,
            "knowledge_cap_utilization_per_mille": final_counts.knowledge * 1_000
                / canwu_technology::TechnologyLimitsV1::canonical().max_knowledge_records,
            "historical_sources": final_counts.historical_sources,
            "historical_practice": final_counts.historical_practice,
            "production_archaeology": final_counts.production_archaeology,
            "per_history_plugin_cap": 1_000,
        },
        "measurements_ms": {
            "build_and_validate": build_ms,
            "ordinary_operation_samples": operation_ms,
            "ordinary_operation_p95": percentile_95(&operation_ms),
            "monthly_boundary_samples": monthly_ms,
            "monthly_boundary_p95": percentile_95(&monthly_ms),
            "snapshot_serialize": snapshot_ms,
            "snapshot_load_and_deep_validate": load_ms,
            "checkpoint_journal_load_and_deep_validate": checkpoint_load_ms,
            "exact_replay_and_deep_validate": replay_ms,
            "disk_write_and_sync": persistence.write_ms,
            "disk_read_after_sync_likely_warm_cache": persistence.read_ms,
        },
        "serialized_bytes": {
            "flat_snapshot": snapshot.len(),
            "current_checkpoint": checkpoint.len(),
            "checkpoint_journal": checkpoint_journal.len(),
            "replay_journal": replay_json.len(),
            "average_flat_snapshot_growth_per_year": annual_snapshot_growth,
        },
        "peak_rss": peak_rss::sample().to_json(),
        "notes": [
            "Each invocation is one fresh process, so peak RSS belongs to one profile.",
            "Each measured month commits one technology claim and one assessment through each optional history plugin.",
            "Persistence timings use actual create/write/sync/read operations in the OS temporary directory.",
            "Snapshot restore and exact replay decode bytes read back from those files; disk read timing is not a cold-cache claim.",
            "This is component evidence on the named machine, not a whole-game 4-core/8-GiB certification.",
            "Historical plugins store bounded assessments only; source bodies remain external."
        ]
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    if let Some(path) = options.output {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{encoded}\n"))?;
    } else {
        println!("{encoded}");
    }
    Ok(())
}

fn apply_monthly_workload(
    canwu: &mut Canwu,
    revision: &canwu_api::DomainRecordVersionRef,
    month: usize,
    at: SimTime,
) -> Result<(), Box<dyn Error>> {
    let actor = PersonId::new(1);
    let request_base = u64::try_from(month)?
        .checked_mul(4)
        .ok_or("monthly request ID overflow")?;
    let expected_revision = canwu.revision();
    apply_tracked_command(
        canwu,
        at,
        request_base + 1,
        expected_revision,
        actor,
        "canwu-technology",
        TECHNOLOGY_COMMAND,
        serde_json::to_value(TechnologyCommandEnvelope {
            id: format!("monthly-technology-{month:04}"),
            subject: KnowledgeHolderRef::Person(actor),
            change: TechnologyRecordChange::Create {
                id: format!("monthly-claim-{month:04}"),
                value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                    asserted_by: KnowledgeHolderRef::Person(actor),
                    proposition: format!("monthly bounded observation {month}"),
                    scope: vec![revision.record.clone()],
                    source_evidence: Vec::new(),
                    relations: Vec::new(),
                    asserted_at: at,
                }),
            },
        })?,
    )?;
    let citation = canwu
        .boundaries()
        .last()
        .map(|boundary| canwu_api::EvidenceRef::Boundary(boundary.id))
        .into_iter()
        .collect::<Vec<_>>();
    let core = AssessmentCore {
        assessor: KnowledgeHolderRef::Person(actor),
        subject: revision.clone(),
        method: "monthly bounded benchmark assessment".to_owned(),
        method_version: "1".to_owned(),
        as_of: at,
        uncertainty_per_mille: 250,
        summary_digest: digest('a'),
        citations: citation,
        contradicts: Vec::new(),
        supersedes: Vec::new(),
    };
    apply_history_command(
        canwu,
        at,
        request_base + 2,
        expected_revision + 1,
        actor,
        HistoricalSourcesAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: format!("monthly-source-{month:04}"),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: HistoricalSourcesAssessmentPayload {
                core: core.clone(),
                earliest_date: at,
                latest_date: at,
                authenticity_per_mille: 800,
                reliability_per_mille: 700,
                provenance_digest: digest('b'),
            },
        },
    )?;
    apply_history_command(
        canwu,
        at,
        request_base + 3,
        expected_revision + 2,
        actor,
        HistoricalPracticeAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: format!("monthly-practice-{month:04}"),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: HistoricalPracticeAssessmentPayload {
                core: core.clone(),
                participants: vec![EntityRef::Person(actor)],
                relation: "monthly controlled reconstruction".to_owned(),
                notebook_digest: Some(digest('c')),
                negative_result: month % 2 == 0,
            },
        },
    )?;
    apply_history_command(
        canwu,
        at,
        request_base + 4,
        expected_revision + 3,
        actor,
        ProductionArchaeologyAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: format!("monthly-production-{month:04}"),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: ProductionArchaeologyAssessmentPayload {
                core,
                observed_kind: "monthly bounded sample".to_owned(),
                observed_digest: digest('d'),
                inferred_process_digest: digest('e'),
                earliest_date: at,
                latest_date: at,
            },
        },
    )?;
    canwu.settle_boundary(BoundaryRequest::at(at))?;
    canwu.settle_boundary(BoundaryRequest::at(at))?;
    Ok(())
}

fn apply_history_command<T: serde::Serialize>(
    canwu: &mut Canwu,
    at: SimTime,
    request_id: u64,
    expected_revision: u64,
    actor: PersonId,
    plugin: &str,
    envelope: T,
) -> Result<(), Box<dyn Error>> {
    apply_tracked_command(
        canwu,
        at,
        request_id,
        expected_revision,
        actor,
        plugin,
        ASSESSMENT_COMMAND,
        serde_json::to_value(envelope)?,
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_tracked_command(
    canwu: &mut Canwu,
    at: SimTime,
    request_id: u64,
    expected_revision: u64,
    actor: PersonId,
    plugin: &str,
    command: &str,
    payload: Value,
) -> Result<(), Box<dyn Error>> {
    canwu.enqueue_command(
        at,
        0,
        CommandRequest::new(
            CommandRequestId::new(request_id),
            expected_revision,
            CommandEnvelope::new(
                Issuer::Actor(actor),
                Command::Plugin {
                    plugin: plugin.to_owned(),
                    command: command.to_owned(),
                    payload,
                },
            )
            .at_time(at),
        ),
    )?;
    Ok(())
}

struct PersistenceMeasurement {
    snapshot: String,
    checkpoint_journal: String,
    replay_journal: ReplayJournal,
    write_ms: f64,
    read_ms: f64,
}

fn measure_file_persistence(
    profile: Profile,
    snapshot: &[u8],
    checkpoint_journal: &[u8],
    replay_journal: &[u8],
) -> Result<PersistenceMeasurement, Box<dyn Error>> {
    let directory = env::temp_dir().join(format!(
        "canwu-technology-profile-{}-{}",
        profile.label(),
        std::process::id()
    ));
    fs::create_dir_all(&directory)?;
    let files = [
        (directory.join("snapshot.json"), snapshot),
        (
            directory.join("checkpoint-journal.json"),
            checkpoint_journal,
        ),
        (directory.join("replay-journal.json"), replay_journal),
    ];
    let write_started = Instant::now();
    for (path, bytes) in &files {
        let mut file = File::create(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let write_ms = milliseconds(write_started.elapsed());
    let read_started = Instant::now();
    let mut loaded = Vec::with_capacity(files.len());
    for (path, _) in &files {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        loaded.push(bytes);
    }
    let read_ms = milliseconds(read_started.elapsed());
    for (path, _) in &files {
        fs::remove_file(path)?;
    }
    fs::remove_dir(&directory)?;
    let snapshot = String::from_utf8(loaded.remove(0))?;
    let checkpoint_journal = String::from_utf8(loaded.remove(0))?;
    let replay_journal = serde_json::from_slice(&loaded.remove(0))?;
    Ok(PersistenceMeasurement {
        snapshot,
        checkpoint_journal,
        replay_journal,
        write_ms,
        read_ms,
    })
}

#[derive(Clone, Copy)]
struct RecordCounts {
    technology: usize,
    knowledge: usize,
    historical_sources: usize,
    historical_practice: usize,
    production_archaeology: usize,
}

impl RecordCounts {
    fn assert_monthly_growth(self, initial: Self, months: usize) -> Result<(), Box<dyn Error>> {
        let expected = |count: usize| count.checked_add(months).ok_or("record count overflow");
        let expected_technology = initial
            .technology
            .checked_add(months.checked_mul(2).ok_or("technology count overflow")?)
            .ok_or("technology count overflow")?;
        if self.technology != expected_technology
            || self.knowledge != expected(initial.knowledge)?
            || self.historical_sources != expected(initial.historical_sources)?
            || self.historical_practice != expected(initial.historical_practice)?
            || self.production_archaeology != expected(initial.production_archaeology)?
        {
            return Err("monthly workload did not commit one technology claim plus its terminal operation, one record per history plugin, and one knowledge publication".into());
        }
        Ok(())
    }
}

fn record_counts(canwu: &Canwu) -> Result<RecordCounts, Box<dyn Error>> {
    Ok(RecordCounts {
        technology: canwu_technology::TechnologyRecordSet::load_host(canwu)?
            .records
            .len(),
        knowledge: canwu
            .knowledge()
            .record_count_in_namespace(canwu_technology::PLUGIN_NAMESPACE),
        historical_sources: count_records::<HistoricalSourcesAssessment>(canwu)?,
        historical_practice: count_records::<HistoricalPracticeAssessment>(canwu)?,
        production_archaeology: count_records::<ProductionArchaeologyAssessment>(canwu)?,
    })
}

fn count_records<T: DomainRecordType>(canwu: &Canwu) -> Result<usize, Box<dyn Error>> {
    let kind = canwu_api::DomainRecordKind::for_type::<T>();
    let mut count = 0usize;
    let mut after = None;
    loop {
        let page = canwu.domain_record_page(&kind, after.as_ref(), 256, Some(canwu.revision()))?;
        let has_more = page.next.is_some();
        after = page.next;
        count = count
            .checked_add(page.records.len())
            .ok_or("record count overflow")?;
        if !has_more {
            return Ok(count);
        }
    }
}

fn measure_operation(
    baseline: &Canwu,
    revision: &canwu_api::DomainRecordVersionRef,
) -> Result<f64, Box<dyn Error>> {
    let mut canwu = baseline.fork();
    let actor = PersonId::new(1);
    let started = Instant::now();
    canwu.enqueue_command(
        canwu.time(),
        0,
        CommandRequest::new(
            CommandRequestId::new(1),
            canwu.revision(),
            CommandEnvelope::new(
                Issuer::Actor(actor),
                Command::Plugin {
                    plugin: "canwu-technology".to_owned(),
                    command: TECHNOLOGY_COMMAND.to_owned(),
                    payload: serde_json::to_value(TechnologyCommandEnvelope {
                        id: "benchmark-operation".to_owned(),
                        subject: KnowledgeHolderRef::Person(actor),
                        change: TechnologyRecordChange::Update {
                            id: "program-00000".to_owned(),
                            expected_version: 1,
                            value: TechnologyRecordPayload::TechnicalProgram(
                                TechnicalProgramPayload {
                                    sponsor: KnowledgeHolderRef::Person(actor),
                                    site: EntityRef::Territory(TerritoryId::new(1)),
                                    revision: Some(revision.clone()),
                                    mode: ProgramMode::Adaptation,
                                    status: ProgramStatus::Paused,
                                    requirements: Vec::new(),
                                    started_at: SimTime::EPOCH,
                                    due_at: None,
                                },
                            ),
                        },
                    })?,
                },
            )
            .at_time(canwu.time()),
        ),
    )?;
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    if canwu
        .domain_record(
            &TypedDomainRecordRef::<TechnicalProgram>::new("program-00000").into_untyped(),
        )
        .is_none_or(|record| record.version != 2)
    {
        return Err("benchmark technology operation did not commit".into());
    }
    Ok(milliseconds(started.elapsed()))
}

fn fixture(
    dimensions: Dimensions,
) -> Result<(Scenario, canwu_api::DomainRecordVersionRef), Box<dyn Error>> {
    let mut records = Vec::new();
    let labels = [
        "papermaking",
        "woodblock",
        "movable-type",
        "gunpowder",
        "steam-engine",
    ];
    let mut revisions = Vec::new();
    for label in labels {
        let metric_id = format!("{label}-metric");
        let technique_id = format!("{label}-technique");
        let revision_id = format!("{label}-revision");
        let application_id = format!("{label}-application");
        let metric = initial_record_version::<MetricSchema>(&metric_id);
        let technique = initial_record_version::<TechniqueSpec>(&technique_id);
        let revision = initial_record_version::<TechniqueRevision>(&revision_id);
        records.push(
            TechnologyCatalogRecord::Metric(MetricSchemaPayload {
                label: format!("{label} local fit"),
                unit: "permille".to_owned(),
                scale: 1_000,
                minimum: 0,
                maximum: 1_000,
            })
            .into_initial_record(metric_id)?,
        );
        let requirements = vec![RequirementGroup {
            id: "local_fit".to_owned(),
            any_of: vec![MetricThreshold {
                id: "minimum_fit".to_owned(),
                metric: metric.clone(),
                comparison: MetricComparison::AtLeast,
                value: 700,
            }],
        }];
        records.push(
            TechnologyCatalogRecord::Technique(TechniqueSpecPayload {
                label: label.to_owned(),
                function: "reference profile".to_owned(),
                requirements: requirements.clone(),
                qualification_rules: Vec::new(),
            })
            .into_initial_record(technique_id)?,
        );
        records.push(
            TechnologyCatalogRecord::Revision(TechniqueRevisionPayload {
                label: format!("{label} reference revision"),
                spec: technique.clone(),
                parents: Vec::new(),
                parameters: Vec::new(),
                evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
                produced_by: None,
                execution_intent: None,
                discovery_evidence: Vec::new(),
            })
            .into_initial_record(revision_id)?,
        );
        records.push(
            TechnologyCatalogRecord::Application(ApplicationSpecPayload {
                label: format!("{label} reference application"),
                technique,
                viability: requirements,
            })
            .into_initial_record(application_id)?,
        );
        revisions.push(revision);
    }

    let government = GovernmentId::new(1);
    let people = (1..=dimensions.sites)
        .map(|index| Person {
            id: person_id(index),
            name: format!("Operator {index}"),
            government,
            current_location: territory_id(index),
            roles: Vec::new(),
            transit: None,
        })
        .collect::<Vec<_>>();
    let territories = (1..=dimensions.sites)
        .map(|index| Territory {
            id: territory_id(index),
            name: format!("Site {index}"),
            controller: government,
            position: MapPoint {
                x: index as f32,
                y: 0.0,
            },
        })
        .collect::<Vec<_>>();

    for index in 0..dimensions.programs {
        let site_index = index % dimensions.sites + 1;
        let revision = revisions[index % revisions.len()].clone();
        records.push(initial_runtime_record::<TechnicalProgram>(
            format!("program-{index:05}"),
            TechnicalProgramPayload {
                sponsor: KnowledgeHolderRef::Person(person_id(site_index)),
                site: EntityRef::Territory(territory_id(site_index)),
                revision: Some(revision.clone()),
                mode: ProgramMode::Adaptation,
                status: ProgramStatus::Active,
                requirements: Vec::new(),
                started_at: SimTime::EPOCH,
                due_at: None,
            },
            "canwu-technology",
            vec![
                core_reference(EntityRef::Person(person_id(site_index))),
                core_reference(EntityRef::Territory(territory_id(site_index))),
                domain_reference(revision.record),
            ],
        )?);
    }
    for index in 0..dimensions.transmissions {
        let destination = index % dimensions.sites + 1;
        let revision = revisions[index % revisions.len()].clone();
        records.push(initial_runtime_record::<TransmissionOpportunity>(
            format!("transmission-{index:05}"),
            TransmissionOpportunityPayload {
                source: Some(KnowledgeHolderRef::Person(PersonId::new(1))),
                source_site: Some(EntityRef::Territory(TerritoryId::new(1))),
                source_capability: None,
                destination: KnowledgeHolderRef::Person(person_id(destination)),
                destination_site: EntityRef::Territory(territory_id(destination)),
                revision: Some(revision.clone()),
                mode: TransmissionMode::DocumentAccess,
                evidence: Vec::new(),
                resulting_program: None,
                opened_at: SimTime::EPOCH,
                active: true,
            },
            "canwu-technology",
            vec![
                core_reference(EntityRef::Person(PersonId::new(1))),
                core_reference(EntityRef::Territory(TerritoryId::new(1))),
                core_reference(EntityRef::Person(person_id(destination))),
                core_reference(EntityRef::Territory(territory_id(destination))),
                domain_reference(revision.record),
            ],
        )?);
    }
    add_history(
        &mut records,
        dimensions.assessments_per_plugin,
        &revisions[0],
    )?;

    let world = WorldSnapshot {
        people,
        governments: vec![Government {
            id: government,
            name: "Benchmark authority".to_owned(),
            capital: TerritoryId::new(1),
        }],
        territories,
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    };
    Ok((
        Scenario {
            start_time: SimTime::EPOCH,
            entities: world.entities(),
            world,
            knowledge: KnowledgeSnapshot::default(),
            domain_records: records,
        },
        revisions[0].clone(),
    ))
}

fn add_history(
    records: &mut Vec<DomainRecord>,
    count: usize,
    subject: &canwu_api::DomainRecordVersionRef,
) -> Result<(), Box<dyn Error>> {
    for index in 0..count {
        let core = AssessmentCore {
            assessor: KnowledgeHolderRef::Person(PersonId::new(1)),
            subject: subject.clone(),
            method: "bounded benchmark assessment".to_owned(),
            method_version: "1".to_owned(),
            as_of: SimTime::EPOCH,
            uncertainty_per_mille: 250,
            summary_digest: digest('a'),
            citations: Vec::new(),
            contradicts: Vec::new(),
            supersedes: Vec::new(),
        };
        let references = vec![
            core_reference(EntityRef::Person(PersonId::new(1))),
            subject_reference(subject.record.clone()),
        ];
        records.push(initial_runtime_record::<HistoricalSourcesAssessment>(
            format!("source-{index:04}"),
            HistoricalSourcesAssessmentPayload {
                core: core.clone(),
                earliest_date: SimTime::EPOCH,
                latest_date: SimTime::EPOCH,
                authenticity_per_mille: 800,
                reliability_per_mille: 700,
                provenance_digest: digest('b'),
            },
            HistoricalSourcesAssessment::PLUGIN_NAME,
            references.clone(),
        )?);
        records.push(initial_runtime_record::<HistoricalPracticeAssessment>(
            format!("practice-{index:04}"),
            HistoricalPracticeAssessmentPayload {
                core: core.clone(),
                participants: vec![EntityRef::Person(PersonId::new(1))],
                relation: "bounded reconstruction".to_owned(),
                notebook_digest: Some(digest('c')),
                negative_result: index % 2 == 0,
            },
            HistoricalPracticeAssessment::PLUGIN_NAME,
            references.clone(),
        )?);
        records.push(initial_runtime_record::<ProductionArchaeologyAssessment>(
            format!("production-{index:04}"),
            ProductionArchaeologyAssessmentPayload {
                core,
                observed_kind: "bounded sample".to_owned(),
                observed_digest: digest('d'),
                inferred_process_digest: digest('e'),
                earliest_date: SimTime::EPOCH,
                latest_date: SimTime::EPOCH,
            },
            ProductionArchaeologyAssessment::PLUGIN_NAME,
            references,
        )?);
    }
    Ok(())
}

fn initial_runtime_record<T: DomainRecordType>(
    id: String,
    payload: T::Payload,
    owner: &str,
    mut references: Vec<DomainReference>,
) -> Result<DomainRecord, Box<dyn Error>>
where
    T::Payload: serde::Serialize,
{
    references.sort();
    references.dedup();
    let mut payload = serde_json::to_value(payload)?;
    if owner == "canwu-technology" {
        payload
            .as_object_mut()
            .ok_or("technology benchmark payload must be an object")?
            .insert(
                PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                serde_json::to_value(PayloadRequiredEvidenceContinuationV1::completed())?,
            );
    }
    Ok(DomainRecord {
        reference: TypedDomainRecordRef::<T>::new(id).into_untyped(),
        owner: owner.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload,
        references,
    })
}

fn domain_reference(record: DomainRecordRef) -> DomainReference {
    DomainReference {
        role: "domain".to_owned(),
        target: DomainReferenceTarget::Domain(record),
    }
}

fn subject_reference(record: DomainRecordRef) -> DomainReference {
    DomainReference {
        role: "subject".to_owned(),
        target: DomainReferenceTarget::Domain(record),
    }
}

fn core_reference(entity: EntityRef) -> DomainReference {
    DomainReference {
        role: "core".to_owned(),
        target: DomainReferenceTarget::Core(entity),
    }
}

fn person_id(index: usize) -> PersonId {
    PersonId::new(u64::try_from(index).expect("fixture person index should fit u64"))
}

fn territory_id(index: usize) -> TerritoryId {
    TerritoryId::new(u64::try_from(index).expect("fixture territory index should fit u64"))
}

fn digest(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn percentile_95(values: &[f64]) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let index = (ordered.len() * 95).div_ceil(100).saturating_sub(1);
    ordered.get(index).copied().unwrap_or_default()
}

fn source_identity() -> Value {
    json!({
        "git_commit": command_output("git", &["rev-parse", "HEAD"]),
        "git_status": command_output("git", &["status", "--short"]),
        "content_hash": source_content_hash(),
        "rustc": command_output("rustc", &["-vV"]),
    })
}

fn source_content_hash() -> String {
    let root = PathBuf::from(command_output("git", &["rev-parse", "--show-toplevel"]));
    let mut files = Vec::new();
    for relative in [
        "Cargo.toml",
        "Cargo.lock",
        "crates/api/canwu-api",
        "crates/extensions/canwu-history-research",
        "crates/model/canwu-knowledge",
        "crates/runtime/canwu-sim",
        "crates/extensions/canwu-technology",
        "benchmarks/performance-harness/Cargo.toml",
        "benchmarks/performance-harness/Cargo.lock",
        "benchmarks/performance-harness/src/bin/technology-profile.rs",
    ] {
        collect_source_files(&root.join(relative), &mut files);
    }
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for path in files {
        if let Ok(relative) = path.strip_prefix(&root)
            && let Ok(bytes) = fs::read(&path)
        {
            hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
            hasher.update(&[0]);
            hasher.update(&bytes);
            hasher.update(&[0]);
        }
    }
    hasher.finalize().to_hex().to_string()
}

fn collect_source_files(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        files.push(path.to_owned());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|name| name == "target" || name == "Cargo.lock")
        {
            continue;
        }
        collect_source_files(&path, files);
    }
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    ProcessCommand::new(program)
        .args(arguments)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|error| format!("unavailable: {error}"))
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        profile: Profile::Interactive,
        samples: 31,
        warmup: 1,
        months: 240,
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
        output: None,
    };
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--profile" => {
                options.profile = match arguments
                    .next()
                    .ok_or("--profile requires a value")?
                    .as_str()
                {
                    "interactive" => Profile::Interactive,
                    "pressure" => Profile::Pressure,
                    value => return Err(format!("unsupported technology profile: {value}").into()),
                };
            }
            "--samples" => {
                options.samples = arguments
                    .next()
                    .ok_or("--samples requires a value")?
                    .parse()?
            }
            "--warmup" => {
                options.warmup = arguments
                    .next()
                    .ok_or("--warmup requires a value")?
                    .parse()?
            }
            "--months" => {
                options.months = arguments
                    .next()
                    .ok_or("--months requires a value")?
                    .parse()?
            }
            "--machine" => {
                options.machine = arguments.next().ok_or("--machine requires a value")?
            }
            "--recorded-on" => {
                options.recorded_on = arguments.next().ok_or("--recorded-on requires a value")?
            }
            "--output" => {
                options.output = Some(PathBuf::from(
                    arguments.next().ok_or("--output requires a path")?,
                ))
            }
            "--help" | "-h" => {
                println!(
                    "Canwu technology home-hardware profile\n\n\
                     --profile interactive|pressure\n\
                     --samples N --warmup N --months N\n\
                     --machine LABEL --recorded-on DATE --output PATH"
                );
                std::process::exit(0);
            }
            value => return Err(format!("unknown argument: {value}").into()),
        }
    }
    if options.samples == 0 || options.months == 0 {
        return Err("--samples and --months must be positive".into());
    }
    Ok(options)
}
