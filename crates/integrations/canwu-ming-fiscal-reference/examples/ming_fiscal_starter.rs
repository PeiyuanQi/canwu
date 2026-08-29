use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, new_ming_fiscal_reference, run_ming_fiscal_sample_cycle,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "hongwu-1391".to_owned());
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, &fixture_id)?;
    run_ming_fiscal_sample_cycle(&mut canwu, &format!("starter.{fixture_id}"))?;
    println!(
        "fixture={} checkpoint={}",
        fixture_id,
        canwu.checkpoint_hash()
    );
    Ok(())
}
