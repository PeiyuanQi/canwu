//! The engagement engine: a reusable, period-neutral Canwu extension that
//! resolves fights between typed forces at a host-owned site under a
//! host-installed rulebook and a [`StepResolver`].
//!
//! The extension owns forces as combat knows them, stepped engagements,
//! sieges, contested crossings, their summaries, and holder-relative
//! engagement reports. It owns no doctrine, unit catalogue, terrain table,
//! weather, control or occupation state, store, or report text: every number
//! that decides a fight is a parameter of the host's [`CombatRulebookV1`],
//! every condition is a host input, and every outcome is a record or a
//! signal the host applies.
//!
//! Every keyed draw the plugin makes cites the admitted step packet as its
//! evidence, so a live, sealed, restored, and replayed run accept it alike.
//! A refused command is a recorded rejection
//! ([`MILITARY_REJECTION_EVENT`]); a step the plugin cannot take holds its
//! engagement ([`ENGAGEMENT_HELD_EVENT`]); neither fails the boundary.

#![allow(
    clippy::default_trait_access,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::wildcard_imports
)]

mod command;
mod crossing;
mod engagement;
mod force;
mod ids;
mod plugin;
mod report;
mod resolver;
mod rulebook;
mod settle;
mod siege;

pub use command::*;
pub use crossing::*;
pub use engagement::*;
pub use force::*;
pub use ids::*;
pub use plugin::*;
pub use report::*;
pub use resolver::*;
pub use rulebook::*;
pub use siege::*;

pub const PLUGIN_NAME: &str = "canwu-military";
pub const PLUGIN_NAMESPACE: &str = "canwu.military";
