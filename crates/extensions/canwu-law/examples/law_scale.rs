use canwu_api::{
    Canwu, DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordRef, Scenario,
    SimTime,
};
use canwu_law::{
    CulturalTargetGenerationRef, LawPlugin, LegalDefinition, LegalMutation, LegalRetirement,
    LegalRuntime, PLUGIN_NAME, PLUGIN_NAMESPACE, compile_law, enqueue_legal_mutation,
    format8_legal_temporal_scale_probe,
};
use std::hint::black_box;
use std::time::Instant;

const SCALES: [usize; 3] = [1_000, 10_000, 100_000];
const SETTLEMENT_SAMPLES: usize = 5_000;
const KERNEL_SAMPLES: usize = 5;
const COLD_HISTORY_SCALES: [u64; 2] = [100_000, 1_000_000];
const COLD_HISTORY_SAMPLES: usize = 50;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut definition = LegalDefinition::new("law-scale");
    definition.budgets.max_retirements = SCALES[SCALES.len() - 1] + KERNEL_SAMPLES;
    let plan = compile_law(&definition)?;
    match std::env::args().nth(1).as_deref() {
        Some("cold") => run_cold_history_probe(&plan),
        Some("hot") => {
            run_local_settlement_probe(&plan)?;
            run_kernel_boundary_probe(&plan)
        }
        Some(mode) => Err(format!("unknown law scale mode: {mode}").into()),
        None => {
            run_local_settlement_probe(&plan)?;
            run_kernel_boundary_probe(&plan)?;
            run_cold_history_probe(&plan)
        }
    }
}

fn run_local_settlement_probe(
    plan: &canwu_law::CompiledLawPlan,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("law-local incremental settlement (excludes persistence encoding)");
    println!("history_records,settlements,total_us,median_ns_per_settlement");
    for scale in SCALES {
        let mut samples = Vec::with_capacity(SETTLEMENT_SAMPLES);
        let mut runtime = historical_runtime(plan, scale);
        runtime.validate_against_plan(plan)?;

        for boundary in 1..=SETTLEMENT_SAMPLES {
            let started = Instant::now();
            let result = runtime.settle_boundary(
                plan,
                SimTime::from_minutes(i64::try_from(boundary)?),
                &[],
            )?;
            black_box(result);
            samples.push(started.elapsed().as_nanos());
        }

        samples.sort_unstable();
        let total_ns = samples.iter().sum::<u128>();
        println!(
            "{scale},{SETTLEMENT_SAMPLES},{},{}",
            total_ns / 1_000,
            samples[samples.len() / 2]
        );
    }
    Ok(())
}

fn run_kernel_boundary_probe(
    plan: &canwu_law::CompiledLawPlan,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "whole Canwu plugin boundary (includes shard decode, COW CAS, and changed-shard encode)"
    );
    println!("history_records,boundaries,total_us,median_us_per_boundary");
    for scale in SCALES {
        let runtime = historical_runtime(plan, scale);
        runtime.validate_against_plan(plan)?;
        let initial = runtime
            .to_record_drafts()?
            .into_iter()
            .map(|draft| DomainRecord {
                reference: draft.reference,
                owner: PLUGIN_NAME.to_owned(),
                class: DomainRecordClass::Record,
                version: 1,
                lifecycle: DomainRecordLifecycle::Active,
                payload: draft.payload,
                references: draft.references,
            })
            .collect();
        let mut canwu = Canwu::new_with_plugins(
            7,
            Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(initial),
            &[&LawPlugin],
        )?;
        let mut samples = Vec::with_capacity(KERNEL_SAMPLES);
        for sample in 0..KERNEL_SAMPLES {
            enqueue_legal_mutation(
                &mut canwu,
                &LegalMutation::RetireCulturalTarget {
                    target: CulturalTargetGenerationRef {
                        target: format!("scale-target:{scale}:{sample}"),
                        generation: 1,
                    },
                    reason: "scale probe".to_owned(),
                },
            )?;
            let started = Instant::now();
            black_box(canwu.step_canonical()?.ok_or("missing legal boundary")?);
            samples.push(started.elapsed().as_micros());
        }
        samples.sort_unstable();
        println!(
            "{scale},{KERNEL_SAMPLES},{},{}",
            samples.iter().sum::<u128>(),
            samples[samples.len() / 2]
        );
    }
    Ok(())
}

fn run_cold_history_probe(
    plan: &canwu_law::CompiledLawPlan,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("root-only cold history (ordinary dirty-shard boundary)");
    println!("cold_records,boundaries,median_us,p95_us");
    let mut cold_results = Vec::new();
    for scale in COLD_HISTORY_SCALES {
        let authenticated_index = format8_legal_temporal_scale_probe(usize::try_from(scale)?)?;
        let mut runtime = LegalRuntime::new(plan);
        let shard = authenticated_index.archive_head.shard.clone();
        runtime.storage.archived_membership_materialized = false;
        runtime
            .storage
            .directory
            .archive_only_shards
            .insert(shard.clone());
        runtime
            .storage
            .archive_heads
            .insert(shard.clone(), authenticated_index.archive_head);
        runtime.storage.validate()?;
        runtime.reaccount_state_budget()?;
        let initial = runtime
            .to_record_drafts()?
            .into_iter()
            .map(|draft| DomainRecord {
                reference: draft.reference,
                owner: PLUGIN_NAME.to_owned(),
                class: DomainRecordClass::Record,
                version: 1,
                lifecycle: DomainRecordLifecycle::Active,
                payload: draft.payload,
                references: draft.references,
            })
            .collect();
        let mut canwu = Canwu::new_with_plugins(
            11,
            Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(initial),
            &[&LawPlugin],
        )?;
        let mut samples = Vec::with_capacity(COLD_HISTORY_SAMPLES);
        for sample in 0..COLD_HISTORY_SAMPLES {
            enqueue_legal_mutation(
                &mut canwu,
                &LegalMutation::RetireCulturalTarget {
                    target: CulturalTargetGenerationRef {
                        target: format!("cold-scale-target:{scale}:{sample}"),
                        generation: 1,
                    },
                    reason: "root-only cold-history scale probe".to_owned(),
                },
            )?;
            let started = Instant::now();
            black_box(canwu.step_canonical()?.ok_or("missing legal boundary")?);
            samples.push(started.elapsed().as_micros());
        }
        samples.sort_unstable();
        let p95 = samples[(samples.len() * 95).div_ceil(100).saturating_sub(1)];
        println!(
            "{scale},{COLD_HISTORY_SAMPLES},{},{}",
            samples[samples.len() / 2],
            p95
        );
        cold_results.push((samples[samples.len() / 2], p95));
    }
    let (baseline_median, baseline_p95) = cold_results[0];
    let (million_median, million_p95) = cold_results[1];
    if million_median > 10_000
        || million_p95 > 16_700
        || million_p95 > baseline_p95.saturating_mul(110).div_ceil(100)
    {
        return Err(format!(
            "root-only cold-history gate failed: baseline={baseline_median}/{baseline_p95}us, million={million_median}/{million_p95}us"
        )
        .into());
    }
    Ok(())
}

fn historical_runtime(plan: &canwu_law::CompiledLawPlan, scale: usize) -> LegalRuntime {
    let mut runtime = LegalRuntime::new(plan);
    runtime.retirements = (0..scale)
        .map(|index| LegalRetirement {
            id: format!("retirement:{index:06}"),
            kind: "historical_legal_record".to_owned(),
            record: DomainRecordRef::new(
                PLUGIN_NAMESPACE,
                "cultural_target",
                format!("historical-target:{index:06}"),
            ),
            cultural_target: None,
            retired_at: SimTime::EPOCH,
            successor: None,
            reason: "scale fixture".to_owned(),
            evidence: Vec::new(),
        })
        .collect();
    runtime
        .reaccount_state_budget()
        .expect("scale fixture must fit the configured state budget");
    runtime
}
