//! Conserved resource accounts, deterministic allocation, transfer escrow,
//! fulfillment evidence, and holder-relative reporting for Canwu.
//!
//! The extension owns physical resource truth. It deliberately does not own
//! production recipes, markets, prices, military readiness, or historical
//! capability content.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]
#![allow(
    clippy::assigning_clones,
    clippy::large_enum_variant,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    clippy::wildcard_imports
)]

mod archive;
mod lease;
mod model;
mod plugin;
mod query;
mod runtime;

pub use archive::*;
pub use lease::*;
pub use model::*;
pub use plugin::*;
pub use query::*;
pub use runtime::*;

/// Persisted plugin identity.
pub const PLUGIN_NAME: &str = "canwu-resource";
/// Namespace used by resource domain and holder-relative knowledge records.
pub const PLUGIN_NAMESPACE: &str = "canwu.resource";
