//! Immutable source, model-card, coverage, and profile content for Canwu's
//! economy reference packages.
//!
//! This crate is intentionally behavioral-code free. Runtime packages may use
//! only definitions materialized by [`compile_content_pack`]. Unknown and
//! not-applicable coverage cells never authorize runtime behavior.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

mod compiler;
mod fixtures;
mod model;

pub use compiler::compile_content_pack;
pub use fixtures::{
    china_industrialization_fixture, fixture_ids, ming_workshop_fixture, synthetic_grain_fixture,
};
pub use model::*;

pub const CONTENT_SCHEMA_VERSION: u32 = 1;
pub const MAX_COVERAGE_CELLS: usize = 16_384;
pub const MAX_MODEL_CARDS: usize = 16_384;
pub const MAX_PROFILES: usize = 4_096;
pub const MAX_CITATIONS_PER_MODEL_CARD: usize = 32;
pub const MAX_CITATION_LOCATOR_BYTES: usize = 8_192;
pub const MAX_COMPILED_PACK_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_AUTHORITATIVE_CONTENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_REFERENCES_PER_DEFINITION: usize = 128;

pub const PACK_ID: &str = "canwu.economy:reference-content";
