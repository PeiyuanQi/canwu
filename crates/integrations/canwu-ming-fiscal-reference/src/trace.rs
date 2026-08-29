//! Human-readable, replay-friendly trace capture for the Ming fiscal reference.

use crate::invalid_reference;
use canwu_api::{
    BoundaryReceipt, BoundaryRecord, Canwu, CanwuError, ErrorCode, KnowledgeQuery, SimTime,
};
use canwu_fiscal::{
    FiscalHistoricalMode, FiscalProjection, FiscalState, FiscalStateRecord,
    fiscal_report_knowledge_schema_id, fiscal_state_reference,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

pub const TRACE_FORMAT_VERSION: u32 = 1;
pub const DEFAULT_TRACE_DIRECTORY: &str = "artifacts/traces";
pub const TRACE_MANIFEST_FILE: &str = "manifest.json";
pub const TRACE_STEPS_FILE: &str = "steps.jsonl";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MingFiscalTracePhase {
    InitialState,
    OpenAssessment,
    AuthorizeExecution,
    AdapterEvidence,
    FiscalExecutionReceipt,
    ReportMaterialization,
    CanonicalBoundary,
}

impl Display for MingFiscalTracePhase {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InitialState => "initial_state",
            Self::OpenAssessment => "open_assessment",
            Self::AuthorizeExecution => "authorize_execution",
            Self::AdapterEvidence => "adapter_evidence",
            Self::FiscalExecutionReceipt => "fiscal_execution_receipt",
            Self::ReportMaterialization => "report_materialization",
            Self::CanonicalBoundary => "canonical_boundary",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MingFiscalTraceCounts {
    pub adoptions: usize,
    pub assessments: usize,
    pub remissions: usize,
    pub execution_requests: usize,
    pub execution_receipts: usize,
    pub audits: usize,
    pub action_outcomes: usize,
    pub transition_candidates: usize,
    pub aggregates: usize,
}

impl From<&FiscalState> for MingFiscalTraceCounts {
    fn from(state: &FiscalState) -> Self {
        Self {
            adoptions: state.adoptions.len(),
            assessments: state.assessments.len(),
            remissions: state.remissions.len(),
            execution_requests: state.execution_requests.len(),
            execution_receipts: state.execution_receipts.len(),
            audits: state.audits.len(),
            action_outcomes: state.action_outcomes.len(),
            transition_candidates: state.transition_candidates.len(),
            aggregates: state.aggregates.len(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MingFiscalTraceFiscalState {
    pub procedure_revision: u64,
    pub historical_year: i32,
    pub historical_mode: FiscalHistoricalMode,
    pub counts: MingFiscalTraceCounts,
    pub state: FiscalState,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projections: BTreeMap<String, FiscalProjection>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MingFiscalTraceFrame {
    pub format_version: u32,
    pub sequence: usize,
    pub phase: MingFiscalTracePhase,
    pub receipt: BoundaryReceipt,
    pub boundary: BoundaryRecord,
    pub revision: u64,
    pub checkpoint_hash: String,
    pub fiscal: MingFiscalTraceFiscalState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MingFiscalTraceManifest {
    pub format_version: u32,
    pub engine_version: String,
    pub fixture_id: String,
    pub seed: u64,
    pub status: String,
    pub started_at: SimTime,
    pub last_settled_at: Option<SimTime>,
    pub step_count: usize,
    pub last_checkpoint_hash: Option<String>,
    pub final_checkpoint_hash: Option<String>,
    pub steps_file: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MingFiscalTracePaths {
    pub directory: PathBuf,
    pub manifest: PathBuf,
    pub steps: PathBuf,
}

#[derive(Debug)]
pub enum TraceDumpError {
    Io(io::Error),
    Json(serde_json::Error),
}

impl Display for TraceDumpError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "trace I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "trace JSON encoding failed: {error}"),
        }
    }
}

impl std::error::Error for TraceDumpError {}

impl From<io::Error> for TraceDumpError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for TraceDumpError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub struct MingFiscalTraceWriter {
    paths: MingFiscalTracePaths,
    steps: BufWriter<File>,
    manifest: MingFiscalTraceManifest,
}

impl MingFiscalTraceWriter {
    pub fn create(
        fixture_id: &str,
        seed: u64,
        started_at: SimTime,
    ) -> Result<Self, TraceDumpError> {
        Self::create_in(default_trace_directory(), fixture_id, seed, started_at)
    }

    pub fn create_in(
        root: impl AsRef<Path>,
        fixture_id: &str,
        seed: u64,
        started_at: SimTime,
    ) -> Result<Self, TraceDumpError> {
        let directory = root
            .as_ref()
            .join("ming-fiscal-reference")
            .join(safe_path_component(fixture_id));
        fs::create_dir_all(&directory)?;
        let manifest = directory.join(TRACE_MANIFEST_FILE);
        let steps_path = directory.join(TRACE_STEPS_FILE);
        let steps_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&steps_path)?;
        let writer = Self {
            paths: MingFiscalTracePaths {
                directory,
                manifest,
                steps: steps_path,
            },
            steps: BufWriter::new(steps_file),
            manifest: MingFiscalTraceManifest {
                format_version: TRACE_FORMAT_VERSION,
                engine_version: Canwu::version().to_owned(),
                fixture_id: fixture_id.to_owned(),
                seed,
                status: "running".to_owned(),
                started_at,
                last_settled_at: None,
                step_count: 0,
                last_checkpoint_hash: None,
                final_checkpoint_hash: None,
                steps_file: TRACE_STEPS_FILE.to_owned(),
            },
        };
        writer.persist_manifest()?;
        Ok(writer)
    }

    #[must_use]
    pub const fn paths(&self) -> &MingFiscalTracePaths {
        &self.paths
    }

    pub fn write_frame(&mut self, frame: &MingFiscalTraceFrame) -> Result<(), TraceDumpError> {
        serde_json::to_writer(&mut self.steps, frame)?;
        self.steps.write_all(b"\n")?;
        self.steps.flush()?;
        self.manifest.step_count = self.manifest.step_count.saturating_add(1);
        self.manifest.last_settled_at = Some(frame.receipt.settled_at);
        self.manifest.last_checkpoint_hash = Some(frame.checkpoint_hash.clone());
        self.persist_manifest()?;
        Ok(())
    }

    pub fn finish(&mut self, canwu: &Canwu) -> Result<MingFiscalTracePaths, TraceDumpError> {
        self.steps.flush()?;
        "complete".clone_into(&mut self.manifest.status);
        self.manifest.final_checkpoint_hash = Some(canwu.checkpoint_hash().to_owned());
        self.manifest.last_checkpoint_hash = Some(canwu.checkpoint_hash().to_owned());
        self.persist_manifest()?;
        Ok(self.paths.clone())
    }

    fn persist_manifest(&self) -> Result<(), TraceDumpError> {
        let file = File::create(&self.paths.manifest)?;
        serde_json::to_writer_pretty(file, &self.manifest)?;
        Ok(())
    }
}

pub fn capture_ming_fiscal_trace_frame(
    canwu: &Canwu,
    sequence: usize,
    phase: MingFiscalTracePhase,
    receipt: BoundaryReceipt,
) -> Result<MingFiscalTraceFrame, CanwuError> {
    let boundary = canwu
        .boundaries()
        .iter()
        .find(|record| record.id == receipt.boundary_id)
        .cloned()
        .ok_or_else(|| invalid_reference("trace receipt has no persisted boundary record"))?;
    let state_record = canwu
        .typed_domain_record(&fiscal_state_reference())
        .ok_or_else(|| invalid_reference("Ming fiscal state is unavailable for trace capture"))?;
    let state = state_record.decode_payload::<FiscalStateRecord>()?;
    let projections = collect_projections(canwu, &state);
    Ok(MingFiscalTraceFrame {
        format_version: TRACE_FORMAT_VERSION,
        sequence,
        phase,
        receipt,
        boundary,
        revision: canwu.revision(),
        checkpoint_hash: canwu.checkpoint_hash().to_owned(),
        fiscal: MingFiscalTraceFiscalState {
            procedure_revision: state.procedure_revision,
            historical_year: state.historical_context.year,
            historical_mode: state.historical_context.mode,
            counts: MingFiscalTraceCounts::from(&state),
            state,
            projections,
        },
    })
}

#[must_use]
pub fn trace_error(error: &TraceDumpError) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidPayload, error.to_string())
}

#[must_use]
pub fn default_trace_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("CANWU_TRACE_DIR") {
        return PathBuf::from(path);
    }
    let mut current = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if current.join("Cargo.toml").is_file() && current.join("crates").is_dir() {
            return current.join(DEFAULT_TRACE_DIRECTORY);
        }
        if !current.pop() {
            break;
        }
    }
    PathBuf::from(DEFAULT_TRACE_DIRECTORY)
}

fn collect_projections(canwu: &Canwu, state: &FiscalState) -> BTreeMap<String, FiscalProjection> {
    let query = KnowledgeQuery {
        schemas: vec![fiscal_report_knowledge_schema_id()],
        ..KnowledgeQuery::default()
    };
    state
        .observer_bindings
        .iter()
        .filter_map(|(observer_id, binding)| {
            let viewer = canwu.viewer_for_actor(binding.actor).ok()?;
            let result = viewer.query_knowledge(&query).ok()?;
            result.records.into_iter().find_map(|record| {
                serde_json::from_value::<FiscalProjection>(record.payload)
                    .ok()
                    .map(|projection| (observer_id.clone(), projection))
            })
        })
        .collect()
}

fn safe_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_owned()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEFAULT_SEED, new_ming_fiscal_reference, run_ming_fiscal_sample_cycle_with_trace};

    #[test]
    fn writer_persists_stable_manifest_and_jsonl_layout() {
        let root = std::env::temp_dir().join(format!(
            "canwu-ming-fiscal-trace-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("test trace root should be creatable");
        let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391")
            .expect("reference runtime should initialize");
        let mut writer =
            MingFiscalTraceWriter::create_in(&root, "hongwu-1391", DEFAULT_SEED, canwu.time())
                .expect("trace writer should initialize");
        let mut sequence = 0;
        run_ming_fiscal_sample_cycle_with_trace(
            &mut canwu,
            "test.trace",
            |canwu, phase, receipt| {
                let frame =
                    capture_ming_fiscal_trace_frame(canwu, sequence, phase, receipt.clone())?;
                sequence += 1;
                writer
                    .write_frame(&frame)
                    .map_err(|error| trace_error(&error))
            },
        )
        .expect("sample trace should settle");
        let paths = writer.finish(&canwu).expect("trace should finalize");

        let manifest: MingFiscalTraceManifest = serde_json::from_str(
            &std::fs::read_to_string(&paths.manifest).expect("manifest should be readable"),
        )
        .expect("manifest should be valid JSON");
        assert_eq!(manifest.format_version, TRACE_FORMAT_VERSION);
        assert_eq!(manifest.fixture_id, "hongwu-1391");
        assert_eq!(manifest.status, "complete");
        assert_eq!(manifest.step_count, sequence);

        let rows = std::fs::read_to_string(&paths.steps)
            .expect("steps should be readable")
            .lines()
            .map(serde_json::from_str::<MingFiscalTraceFrame>)
            .collect::<Result<Vec<_>, _>>()
            .expect("every JSONL row should decode");
        assert_eq!(rows.len(), sequence);
        assert_eq!(
            rows.first().map(|row| row.phase),
            Some(MingFiscalTracePhase::OpenAssessment)
        );
        assert_eq!(
            rows.last().map(|row| row.phase),
            Some(MingFiscalTracePhase::FiscalExecutionReceipt)
        );
        assert_eq!(
            paths.directory.file_name().and_then(|name| name.to_str()),
            Some("hongwu-1391")
        );

        std::fs::remove_dir_all(root).expect("test trace root should be removable");
    }
}
