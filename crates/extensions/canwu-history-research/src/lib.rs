//! Optional historical research assessment plugins for Canwu.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

mod analysis;
mod model;
mod plugin;

pub use analysis::{
    HistoricalAnalysis, HistoricalAssessmentView, from_historical_research_checkpoint_journal,
    from_historical_research_snapshot_json, replay_historical_research_from_journal,
    validate_historical_research_runtime,
};
pub use model::*;
pub use plugin::{
    ASSESSMENT_COMMAND, ASSESSMENT_INGRESS, HistoricalPracticePlugin, HistoricalResearchSuite,
    HistoricalSourcesPlugin, ProductionArchaeologyPlugin,
};
