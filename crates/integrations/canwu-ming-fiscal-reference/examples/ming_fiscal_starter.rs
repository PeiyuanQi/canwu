use canwu_api::{BoundaryRequest, SimDuration, SystemCadence};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, MingFiscalTracePaths, MingFiscalTraceWriter, capture_ming_fiscal_trace_frame,
    new_ming_fiscal_reference, run_ming_fiscal_sample_cycle_with_trace, start_trace_viewer,
    trace_error,
};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContinuousCadence {
    Daily,
    Monthly,
    Annual,
}

impl ContinuousCadence {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "daily" => Ok(Self::Daily),
            "monthly" => Ok(Self::Monthly),
            "annual" => Ok(Self::Annual),
            _ => Err(format!(
                "unknown cadence {value}; expected daily, monthly, or annual"
            )),
        }
    }

    const fn system_cadence(self) -> SystemCadence {
        match self {
            Self::Daily => SystemCadence::Daily,
            Self::Monthly => SystemCadence::Monthly,
            Self::Annual => SystemCadence::Annual,
        }
    }

    const fn default_step_days(self) -> i64 {
        match self {
            Self::Daily => 1,
            // SimTime is minute-based and intentionally has no Gregorian
            // calendar. These are fixed simulation periods, not claims about
            // month or year length; pass --step-days for a different policy.
            Self::Monthly => 30,
            Self::Annual => 365,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Monthly => "monthly",
            Self::Annual => "annual",
        }
    }
}

struct Options {
    fixture_id: String,
    trace_root: Option<PathBuf>,
    days: u64,
    cadence: ContinuousCadence,
    step_days: Option<i64>,
    open_viewer: bool,
    viewer_port: u16,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1).peekable();
        if matches!(args.peek().map(String::as_str), Some("--help" | "-h")) {
            return Err(help_text().to_owned());
        }
        let fixture_id = if args.peek().is_some_and(|value| !value.starts_with('-')) {
            args.next()
                .expect("peeked fixture argument should remain available")
        } else {
            "hongwu-1391".to_owned()
        };
        let mut options = Self {
            fixture_id,
            trace_root: None,
            days: 0,
            cadence: ContinuousCadence::Daily,
            step_days: None,
            open_viewer: env_flag("CANWU_OPEN_TRACE_VIEWER"),
            viewer_port: 0,
        };

        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--trace-dir" => {
                    options.trace_root =
                        Some(PathBuf::from(args.next().ok_or_else(|| {
                            "--trace-dir requires a directory path".to_owned()
                        })?));
                }
                "--days" => {
                    options.days = args
                        .next()
                        .ok_or_else(|| "--days requires a non-negative integer".to_owned())?
                        .parse()
                        .map_err(|_| "--days requires a non-negative integer".to_owned())?;
                }
                "--cadence" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--cadence requires daily, monthly, or annual".to_owned())?;
                    options.cadence = ContinuousCadence::parse(&value)?;
                }
                "--step-days" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--step-days requires a positive integer".to_owned())?;
                    let step_days = value
                        .parse::<i64>()
                        .map_err(|_| "--step-days requires a positive integer".to_owned())?;
                    if step_days <= 0 {
                        return Err("--step-days requires a positive integer".to_owned());
                    }
                    options.step_days = Some(step_days);
                }
                "--open-viewer" => {
                    options.open_viewer = true;
                }
                "--viewer-port" => {
                    options.viewer_port = args
                        .next()
                        .ok_or_else(|| {
                            "--viewer-port requires a TCP port (0 selects an available port)"
                                .to_owned()
                        })?
                        .parse()
                        .map_err(|_| {
                            "--viewer-port requires a TCP port (0 selects an available port)"
                                .to_owned()
                        })?;
                }
                "--help" | "-h" => return Err(help_text().to_owned()),
                unknown => {
                    return Err(format!("unknown argument {unknown}; use --help for usage"));
                }
            }
        }

        Ok(options)
    }
}

const fn help_text() -> &'static str {
    "usage: ming_fiscal_starter [fixture-id] [options]\n\
\n\
options:\n\
  --trace-dir <path>       override the trace root\n\
  --days <N>               continue for N total simulation days after the sample cycle\n\
  --cadence <kind>         daily, monthly, or annual (default: daily)\n\
  --step-days <N>          fixed simulation-day quantum; overrides cadence default\n\
  --open-viewer             start localhost viewer and open the generated trace\n\
  --viewer-port <N>        viewer TCP port; 0 selects an available port (default)\n\
  --help                   show this help"
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = match Options::parse() {
        Ok(options) => options,
        Err(error) if error == help_text() => {
            println!("{error}");
            return Ok(());
        }
        Err(error) => return Err(format!("{error}\n\n{}", help_text()).into()),
    };
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, &options.fixture_id)?;
    let mut writer = match options.trace_root.as_ref() {
        Some(root) => {
            MingFiscalTraceWriter::create_in(root, &options.fixture_id, DEFAULT_SEED, canwu.time())?
        }
        None => MingFiscalTraceWriter::create(&options.fixture_id, DEFAULT_SEED, canwu.time())?,
    };
    let mut sequence = 0;
    run_ming_fiscal_sample_cycle_with_trace(
        &mut canwu,
        &format!("starter.{}", options.fixture_id),
        |canwu, phase, receipt| {
            let frame = capture_ming_fiscal_trace_frame(canwu, sequence, phase, receipt.clone())?;
            sequence += 1;
            writer
                .write_frame(&frame)
                .map_err(|error| trace_error(&error))
        },
    )?;

    if options.days > 0 {
        let horizon_days = i64::try_from(options.days)
            .map_err(|_| "--days exceeds the supported simulation range")?;
        let default_step_days = options.cadence.default_step_days();
        let configured_step_days = options.step_days.unwrap_or(default_step_days);
        let cadence = options.cadence.system_cadence();
        let mut settled_boundaries = 0usize;
        let mut remaining_days = horizon_days;
        let mut step_index = 0u64;
        while remaining_days > 0 {
            let step_days = remaining_days.min(configured_step_days);
            let step = SimDuration::checked_days(step_days)
                .ok_or("the requested step duration exceeds the supported range")?;
            let next_boundary = canwu
                .time()
                .checked_add(step)
                .ok_or("continuous run exceeds the supported simulation time range")?;
            let complete_cadence_period = step_days == configured_step_days;
            let receipts = if complete_cadence_period {
                canwu.schedule_calendar_boundary(next_boundary, vec![cadence.clone()])?;
                canwu.advance_canonical(step)?
            } else {
                vec![canwu.settle_boundary(BoundaryRequest::at(next_boundary))?]
            };
            settled_boundaries = settled_boundaries.saturating_add(receipts.len());
            for receipt in receipts {
                let frame = capture_ming_fiscal_trace_frame(
                    &canwu,
                    sequence,
                    canwu_ming_fiscal_reference::MingFiscalTracePhase::CanonicalBoundary,
                    receipt,
                )?;
                sequence += 1;
                writer
                    .write_frame(&frame)
                    .map_err(|error| trace_error(&error))?;
            }
            println!(
                "continuous_step={} cadence={} time={} boundaries={}",
                step_index + 1,
                if complete_cadence_period {
                    options.cadence.as_str()
                } else {
                    "none"
                },
                canwu.time(),
                settled_boundaries,
            );
            remaining_days -= step_days;
            step_index += 1;
        }
    }

    let paths = writer.finish(&canwu)?;
    println!(
        "fixture={} checkpoint={} frames={} trace_manifest={} trace_steps={}",
        options.fixture_id,
        canwu.checkpoint_hash(),
        sequence,
        paths.manifest.display(),
        paths.steps.display(),
    );
    open_viewer_if_requested(&options, &paths);
    Ok(())
}

fn open_viewer_if_requested(options: &Options, paths: &MingFiscalTracePaths) {
    if !options.open_viewer {
        return;
    }
    match workspace_root().and_then(|root| {
        start_trace_viewer(&root, &paths.directory, options.viewer_port)
            .map_err(|error| error.to_string())
    }) {
        Ok(viewer) => {
            if let Err(error) = viewer.open_browser() {
                eprintln!("trace_viewer_browser_warning={error}");
            }
            println!("trace_viewer_url={}", viewer.url());
            viewer.wait();
        }
        Err(error) => {
            eprintln!("trace_viewer_warning={error}");
        }
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    )
}

fn workspace_root() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CANWU_WORKSPACE_ROOT") {
        let path = PathBuf::from(path);
        if path
            .join("tools")
            .join("trace-viewer")
            .join("index.html")
            .is_file()
        {
            return Ok(path);
        }
        return Err(format!(
            "CANWU_WORKSPACE_ROOT does not contain tools/trace-viewer/index.html: {}",
            path.display()
        ));
    }
    let mut current = std::env::current_dir().map_err(|error| error.to_string())?;
    loop {
        if current.join("Cargo.toml").is_file()
            && current.join("crates").is_dir()
            && current
                .join("tools")
                .join("trace-viewer")
                .join("index.html")
                .is_file()
        {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }
    Err("could not locate the Canwu workspace root; set CANWU_WORKSPACE_ROOT".to_owned())
}
