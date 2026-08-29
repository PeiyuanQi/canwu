use canwu_api::{
    Canwu, DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordRef, Scenario,
    SimTime,
};
use canwu_law::{
    CulturalTargetGenerationRef, LawPlugin, LegalDefinition, LegalMutation, LegalRetirement,
    LegalRuntime, PLUGIN_NAME, PLUGIN_NAMESPACE, compile_law, enqueue_legal_mutation,
};
use std::hint::black_box;
use std::time::Instant;

const SCALES: [usize; 3] = [1_000, 10_000, 100_000];
const SETTLEMENT_SAMPLES: usize = 5_000;
const KERNEL_SAMPLES: usize = 5;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut definition = LegalDefinition::new("law-scale");
    definition.budgets.max_retirements = SCALES[SCALES.len() - 1] + KERNEL_SAMPLES;
    let plan = compile_law(&definition)?;
    println!("law-local incremental settlement (excludes persistence encoding)");
    println!("history_records,settlements,total_us,median_ns_per_settlement");

    for scale in SCALES {
        let mut samples = Vec::with_capacity(SETTLEMENT_SAMPLES);
        let mut runtime = historical_runtime(&plan, scale);
        runtime.validate_against_plan(&plan)?;

        for boundary in 1..=SETTLEMENT_SAMPLES {
            let started = Instant::now();
            let result = runtime.settle_boundary(
                &plan,
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

    println!("whole Canwu plugin boundary (includes aggregate decode, clone, CAS, and encode)");
    println!("history_records,boundaries,total_us,median_us_per_boundary");
    for scale in SCALES {
        let runtime = historical_runtime(&plan, scale);
        runtime.validate_against_plan(&plan)?;
        let initial = runtime.to_record_draft()?;
        let mut canwu = Canwu::new_with_plugins(
            7,
            Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
                reference: initial.reference,
                owner: PLUGIN_NAME.to_owned(),
                class: DomainRecordClass::Record,
                version: 1,
                lifecycle: DomainRecordLifecycle::Active,
                payload: initial.payload,
                references: initial.references,
            }]),
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
