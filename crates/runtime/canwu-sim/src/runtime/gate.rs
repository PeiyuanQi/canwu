//! Continuation gate hooks (feature `continuation-gate`), for the release
//! test that `canwu-api/continuation-gate` turns on.
//!
//! A host that wraps a live run can restore a continuation checkpoint and
//! replay a journal with the plugin registry of that run, without holding the
//! plugin objects. They prove that the restored or replayed run reaches the
//! same state (checkpoint hash and continuation round trip), not that the
//! public restore and replay paths, which register the plugins and validate
//! their records and activation, accept the checkpoint or the journal. These
//! hooks are not part of the public contract, and applications must never
//! enable the feature.

use super::{
    CanwuError, CommitmentDomains, CompactedSimulation, ContinuationCheckpoint, ErrorCode,
    RandomStreamState, Simulation,
};
use std::sync::Arc;

impl Simulation {
    /// Takes the plugin registry and type schema of `source`. Unlike
    /// registering the same plugins, it neither validates this run's records
    /// for each owner nor calls `validate_activation`, and it replaces the
    /// registry and schema this run carries rather than rehydrating against
    /// them as `from_continuation_checkpoint_with_plugins` does.
    pub(super) fn adopt_registry_of(&mut self, source: &Self) -> Result<(), CanwuError> {
        self.plugins = source.plugins.clone();
        self.schema = source.schema.clone();
        for stream in self.plugins.random_stream_owners.keys() {
            self.state
                .current
                .random_streams
                .entry(stream.clone())
                .or_insert_with(|| {
                    RandomStreamState::initial(self.state.current.root_seed, stream.clone())
                });
        }
        self.invalidate_commitments(
            CommitmentDomains::RANDOM_STREAMS | CommitmentDomains::IDENTITY,
        );
        self.refresh_checkpoint_hash()
    }

    /// Restores `checkpoint`, whose stored hash is `hash`, with this run's
    /// plugin registry and archive provider.
    #[doc(hidden)]
    pub fn gate_restore_continuation(
        &self,
        checkpoint: ContinuationCheckpoint,
        hash: &str,
    ) -> Result<CompactedSimulation, CanwuError> {
        if checkpoint.compute_continuation_hash()? != hash {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "continuation checkpoint does not match the stored continuation hash",
            ));
        }
        let mut simulation = Self::restore_continuation(checkpoint)?;
        simulation.plugin_archive_provider = Arc::clone(&self.plugin_archive_provider);
        simulation.adopt_registry_of(self)?;
        simulation.ensure_runtime_ready()?;
        simulation.verify_continuation_round_trip(hash)?;
        simulation.into_compacted()
    }

    /// Replays this run's journal with this run's plugin registry and
    /// `provider`, or this run's archive provider, and returns the replayed
    /// checkpoint hash.
    #[doc(hidden)]
    pub fn gate_replay_checkpoint_hash(
        &self,
        provider: Option<Arc<dyn super::PluginArchiveObjectProvider + Send + Sync>>,
    ) -> Result<String, CanwuError> {
        let journal = self.replay_journal();
        let provider = provider.unwrap_or_else(|| Arc::clone(&self.plugin_archive_provider));
        let mut replayed = Self::replay_from_journal_activated(
            super::replay::Activation::Registry(self),
            &journal,
            provider,
        )?;
        replayed.plugin_archive_provider = Arc::clone(&self.plugin_archive_provider);
        Ok(replayed.checkpoint_hash().to_owned())
    }
}
