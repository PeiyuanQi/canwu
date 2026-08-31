use canwu_economy_reference::{GrainDecision, GrainHarness};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decisions = [
        GrainDecision::Balanced,
        GrainDecision::ReliefFirst,
        GrainDecision::Balanced,
        GrainDecision::ForceFirst,
        GrainDecision::RequisitionForForce,
        GrainDecision::Balanced,
        GrainDecision::ReliefFirst,
        GrainDecision::Balanced,
        GrainDecision::ForceFirst,
        GrainDecision::Balanced,
        GrainDecision::ReliefFirst,
        GrainDecision::Balanced,
        GrainDecision::ForceFirst,
        GrainDecision::Balanced,
    ];
    let summary = GrainHarness::new()?.run_fourteen_months(decisions)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
