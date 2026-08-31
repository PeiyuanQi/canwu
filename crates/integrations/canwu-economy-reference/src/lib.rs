//! Replaceable economy reference composition for Canwu.
//!
//! G1b supplies a deterministic grain-loop fixture that composes conserved
//! resource accounts with real routing and transport execution types. G5
//! supplies detached, holder-bound local-scarcity and evidence-qualified
//! price-pressure projections. Neither projection is authoritative market
//! state.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

mod economy;
mod grain;
mod plugin;
mod projection;
mod query;

pub use economy::*;
pub use grain::*;
pub use plugin::*;
pub use projection::*;
pub use query::*;

pub const PLUGIN_NAME: &str = "canwu-economy-reference";
pub const PLUGIN_NAMESPACE: &str = "canwu.economy-reference";
pub const REFERENCE_VERSION: &str = "canwu.economy-reference.v1";
pub const MAX_TYPED_SOURCE_ADAPTERS: usize = 8;
pub const MAX_OBSERVATION_FACTS: usize = 256;
pub const MAX_ADAPTER_CALLS: usize = 16;
pub const MAX_PRICE_FACTORS: usize = 32;
