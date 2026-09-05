//! Reusable, period-neutral military domain extension for Canwu.

#![allow(
    clippy::collapsible_if,
    clippy::default_trait_access,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::needless_question_mark,
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    clippy::wildcard_imports
)]

mod model;
mod plugin;

pub use model::*;
pub use plugin::*;

pub const PLUGIN_NAME: &str = "canwu-military";
pub const PLUGIN_NAMESPACE: &str = "canwu.military";
