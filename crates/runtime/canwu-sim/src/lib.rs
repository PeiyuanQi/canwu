//! Deterministic runtime, validated commands, scheduling, plugins, and snapshots.

#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

mod runtime;

pub use runtime::*;
