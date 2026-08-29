use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, MingFiscalTraceWriter, capture_ming_fiscal_trace_frame,
    new_ming_fiscal_reference, run_ming_fiscal_sample_cycle_with_trace, trace_error,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let fixture_id = args.next().unwrap_or_else(|| "hongwu-1391".to_owned());
    let trace_root = match args.next().as_deref() {
        Some("--trace-dir") => Some(std::path::PathBuf::from(
            args.next().ok_or("--trace-dir requires a directory path")?,
        )),
        Some(unknown) => {
            return Err(format!("unknown argument {unknown}; expected --trace-dir <path>").into());
        }
        None => None,
    };
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, &fixture_id)?;
    let mut writer = match trace_root {
        Some(root) => {
            MingFiscalTraceWriter::create_in(root, &fixture_id, DEFAULT_SEED, canwu.time())?
        }
        None => MingFiscalTraceWriter::create(&fixture_id, DEFAULT_SEED, canwu.time())?,
    };
    let mut sequence = 0;
    run_ming_fiscal_sample_cycle_with_trace(
        &mut canwu,
        &format!("starter.{fixture_id}"),
        |canwu, phase, receipt| {
            let frame = capture_ming_fiscal_trace_frame(canwu, sequence, phase, receipt.clone())?;
            sequence += 1;
            writer
                .write_frame(&frame)
                .map_err(|error| trace_error(&error))
        },
    )?;
    let paths = writer.finish(&canwu)?;
    println!(
        "fixture={} checkpoint={} trace_manifest={} trace_steps={}",
        fixture_id,
        canwu.checkpoint_hash(),
        paths.manifest.display(),
        paths.steps.display(),
    );
    Ok(())
}
