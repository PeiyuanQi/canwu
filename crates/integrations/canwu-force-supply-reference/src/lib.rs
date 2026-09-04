//! Replaceable force-supply reference integration built only on public Canwu
//! and `canwu-resource` contracts.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

mod archive;
mod content;
mod model;
mod plugin;
mod query;

pub use archive::*;
pub use content::*;
pub use model::*;
pub use plugin::*;
pub use query::*;

pub const PLUGIN_NAME: &str = "canwu-force-supply-reference";
pub const PLUGIN_NAMESPACE: &str = "canwu.force-supply-reference";
