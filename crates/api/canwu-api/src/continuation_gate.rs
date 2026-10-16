//! Continuation gate, a release test behind the `continuation-gate` feature.
//!
//! The release skill runs the workspace tests with the feature once before
//! each release; CI never enables it. It is not part of the supported API,
//! and applications must never enable it: it runs every mutation twice,
//! restores and replays the run, and panics instead of returning an error.
//!
//! With the feature on, every [`crate::Canwu`] keeps a shadow run: a compact
//! fork made before the first mutation, sealed before every later mutation,
//! and replaced from time to time by a run restored from its own
//! continuation checkpoint. Every mutation runs on both, and the gate panics
//! with a `continuation gate:` message when a result, the checkpoint hash,
//! or the authoritative state hash differs; when the shadow cannot seal or
//! write a continuation checkpoint; when the unsealed run's continuation
//! checkpoint differs from the shadow's; or when an exact replay of the
//! unsealed run reaches a different checkpoint hash.
//!
//! The gate proves state equality (checkpoint hashes and the continuation
//! round trip). It restores and replays with the run's own plugin registry,
//! so it does not prove that the public restore and replay paths, which
//! register the plugins and validate their records and activation, accept
//! the checkpoint or the journal.
//!
//! Plugin handlers and decision policies run twice per call, on the run and
//! on the shadow, and again when the gate replays, so a stateful spy sees
//! extra calls. A `Canwu::fork` of a gated run shares the parent's recording
//! provider, so the fork's archive loads are recorded with the parent's.
//!
//! The shadow has its own archive provider: it answers from what the run
//! loaded in the same call, and from the run's provider only for an object
//! the run did not load, so a test that counts provider loads sees the
//! shadow only when the two read differently. An exact replay of the run
//! answers from everything the run has loaded, and from the run's provider
//! only for an object the run never loaded. A test that deliberately
//! declares evidence after its seal leaves the shadow unable to write a
//! continuation checkpoint or to seal; the one such test,
//! `a_packet_first_declared_after_its_seal_is_not_ready`, expects that panic
//! under the feature.

use canwu_sim::{CanwuError, CompactedSimulation, PluginArchiveObjectProvider, Simulation};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

/// Archive provider answers by namespace and object id.
type Answers = BTreeMap<(String, String), Option<Vec<u8>>>;

/// The run's archive provider, recording what each call loads.
struct RecordingProvider {
    inner: Arc<dyn PluginArchiveObjectProvider + Send + Sync>,
    /// What the current call loaded.
    loads: Mutex<Answers>,
    /// The latest answer for every object the run has loaded.
    history: Mutex<Answers>,
}

impl PluginArchiveObjectProvider for RecordingProvider {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        let loaded = self
            .inner
            .load_plugin_archive_object(namespace, object_id)?;
        let key = (namespace.to_owned(), object_id.to_owned());
        self.loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.clone(), loaded.clone());
        self.history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, loaded.clone());
        Ok(loaded)
    }
}

/// The shadow's archive provider: what the run loaded in this call, then
/// the run's own provider.
struct ShadowProvider(Arc<RecordingProvider>);

impl PluginArchiveObjectProvider for ShadowProvider {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        if let Some(loaded) = self
            .0
            .loads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(namespace.to_owned(), object_id.to_owned()))
        {
            return Ok(loaded.clone());
        }
        self.0
            .inner
            .load_plugin_archive_object(namespace, object_id)
    }
}

/// The provider of an exact replay of the run: everything the run has
/// loaded, then the run's own provider.
struct ReplayProvider(Arc<RecordingProvider>);

impl PluginArchiveObjectProvider for ReplayProvider {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        if let Some(loaded) = self
            .0
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(namespace.to_owned(), object_id.to_owned()))
        {
            return Ok(loaded.clone());
        }
        self.0
            .inner
            .load_plugin_archive_object(namespace, object_id)
    }
}

/// The shadow run and the schedule of its restores.
#[derive(Default)]
pub(crate) struct Gate {
    /// Boxed, so a gated `Canwu` stays about the size of an ungated one.
    shadow: Option<Box<CompactedSimulation>>,
    /// The run's provider, when the host attached one through the gate.
    provider: Option<Arc<RecordingProvider>>,
    /// Restore the shadow once it has settled this many boundaries.
    next_restore: u64,
    /// Replay the unsealed run once the shadow has settled this many
    /// boundaries.
    next_replay: u64,
}

fn fail(message: &str) -> ! {
    panic!("continuation gate: {message}")
}

fn first_difference(left: &str, right: &str) -> usize {
    left.bytes()
        .zip(right.bytes())
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| left.len().min(right.len()))
}

fn excerpt(text: &str, at: usize) -> &str {
    let mut start = at.saturating_sub(200).min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + 400).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    &text[start..end]
}

impl Gate {
    /// The shadow, made from `primary` if there is none, and sealed.
    pub(crate) fn prepare(&mut self, primary: &Simulation) -> Option<&mut CompactedSimulation> {
        if self.shadow.is_none() {
            let mut shadow = primary
                .fork()
                .into_compacted()
                .unwrap_or_else(|error| fail(&format!("no compact fork: {error:?}")));
            if let Some(provider) = &self.provider {
                shadow.set_plugin_archive_object_provider(Arc::new(ShadowProvider(Arc::clone(
                    provider,
                ))));
            }
            self.shadow = Some(Box::new(shadow));
            if self.next_restore == 0 {
                self.next_restore = 2;
                self.next_replay = 3;
            }
        }
        if let Some(provider) = &self.provider {
            provider
                .loads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }
        let shadow = self.shadow.as_deref_mut()?;
        if let Err(error) = shadow.seal_evidence() {
            fail(&format!(
                "the sealed run cannot seal at revision {}: {error:?}",
                shadow.revision()
            ));
        }
        Some(shadow)
    }

    /// Records what the run loads through `provider`, which the run gets
    /// back, and gives the shadow its own provider.
    pub(crate) fn wrap_provider(
        &mut self,
        provider: Arc<dyn PluginArchiveObjectProvider + Send + Sync>,
    ) -> Arc<dyn PluginArchiveObjectProvider + Send + Sync> {
        let recording = Arc::new(RecordingProvider {
            inner: provider,
            loads: Mutex::new(BTreeMap::new()),
            history: Mutex::new(BTreeMap::new()),
        });
        if let Some(shadow) = self.shadow.as_mut() {
            shadow.set_plugin_archive_object_provider(Arc::new(ShadowProvider(Arc::clone(
                &recording,
            ))));
        }
        self.provider = Some(Arc::clone(&recording));
        recording
    }

    /// Forgets the shadow; the next mutation makes a new one.
    pub(crate) fn drop_shadow(&mut self) {
        self.shadow = None;
    }

    pub(crate) fn shadow(&mut self) -> Option<&mut CompactedSimulation> {
        self.shadow.as_deref_mut()
    }

    /// Compares one mutation's results and the runs' hashes, then restores
    /// or replays when the schedule says so.
    pub(crate) fn check<T: Debug>(
        &mut self,
        method: &str,
        primary: &Simulation,
        unsealed: &Result<T, CanwuError>,
        sealed: Option<Result<T, CanwuError>>,
    ) {
        let Some(sealed) = sealed else {
            return;
        };
        let Some(shadow) = self.shadow.as_mut() else {
            return;
        };
        // Host errors are compared by code: a message may say why the
        // evidence is unavailable in words that depend on the seal.
        let (unsealed_text, sealed_text) = match (unsealed, &sealed) {
            (Err(left), Err(right)) => (format!("{:?}", left.code), format!("{:?}", right.code)),
            _ => (format!("{unsealed:?}"), format!("{sealed:?}")),
        };
        if unsealed_text != sealed_text {
            let at = first_difference(&unsealed_text, &sealed_text);
            let records = (
                format!("{:?}", primary.boundaries().last()),
                format!("{:?}", shadow.boundaries().last()),
            );
            let record_at = first_difference(&records.0, &records.1);
            fail(&format!(
                "{method} diverged at revision {}:\n unsealed …{}…\n sealed   …{}…\n last boundary records:\n unsealed …{}…\n sealed   …{}…",
                primary.revision(),
                excerpt(&unsealed_text, at),
                excerpt(&sealed_text, at),
                excerpt(&records.0, record_at),
                excerpt(&records.1, record_at)
            ));
        }
        if primary.checkpoint_hash() != shadow.checkpoint_hash() {
            fail(&format!(
                "{method} left different checkpoint hashes at revision {}",
                primary.revision()
            ));
        }
        let hashes = (
            primary.authoritative_state_hash(),
            shadow.authoritative_state_hash(),
        );
        if format!("{:?}", hashes.0) != format!("{:?}", hashes.1) {
            fail(&format!(
                "{method} left different state hashes at revision {}: {:?}",
                primary.revision(),
                hashes
            ));
        }
        let boundaries = shadow
            .evidence_cursor()
            .unwrap_or_else(|error| fail(&format!("no evidence cursor: {error:?}")))
            .boundary_count;
        if boundaries >= self.next_restore {
            self.restore(method, primary);
            self.next_restore = boundaries.saturating_mul(2).max(boundaries + 1);
        }
        if boundaries >= self.next_replay {
            let provider = self.provider.as_ref().map(|provider| {
                Arc::new(ReplayProvider(Arc::clone(provider)))
                    as Arc<dyn PluginArchiveObjectProvider + Send + Sync>
            });
            let replayed = primary
                .gate_replay_checkpoint_hash(provider)
                .unwrap_or_else(|error| {
                    fail(&format!(
                        "the unsealed run does not replay after {method} at revision {}: {error:?}",
                        primary.revision()
                    ))
                });
            if replayed != primary.checkpoint_hash() {
                fail(&format!(
                    "the replay of the unsealed run differs after {method} at revision {}",
                    primary.revision()
                ));
            }
            self.next_replay = boundaries.saturating_mul(4).max(boundaries + 1);
        }
    }

    /// Replaces the shadow by a run restored from its own continuation
    /// checkpoint, after checking that the unsealed run writes the same one.
    fn restore(&mut self, method: &str, primary: &Simulation) {
        let Some(shadow) = self.shadow.as_mut() else {
            return;
        };
        let (checkpoint, hash) = shadow.continuation_checkpoint().unwrap_or_else(|error| {
            fail(&format!(
                "the sealed run writes no continuation checkpoint after {method} at revision {}: {error:?}",
                primary.revision()
            ))
        });
        let (unsealed, unsealed_hash) = primary.continuation_checkpoint().unwrap_or_else(|error| {
            fail(&format!(
                "the unsealed run writes no continuation checkpoint after {method} at revision {}: {error:?}",
                primary.revision()
            ))
        });
        if unsealed_hash != hash {
            let unsealed_text = unsealed.to_json().unwrap_or_default();
            let sealed_text = checkpoint.to_json().unwrap_or_default();
            let at = first_difference(&unsealed_text, &sealed_text);
            fail(&format!(
                "continuation checkpoints differ after {method} at revision {}:\n unsealed …{}…\n sealed   …{}…",
                primary.revision(),
                excerpt(&unsealed_text, at),
                excerpt(&sealed_text, at)
            ));
        }
        let restored = shadow
            .gate_runtime()
            .gate_restore_continuation(checkpoint, &hash)
            .unwrap_or_else(|error| {
                fail(&format!(
                    "the continuation checkpoint does not restore after {method} at revision {}: {error:?}",
                    primary.revision()
                ))
            });
        if restored.checkpoint_hash() != primary.checkpoint_hash() {
            fail(&format!(
                "the restored run has a different checkpoint hash after {method} at revision {}",
                primary.revision()
            ));
        }
        self.shadow = Some(Box::new(restored));
    }
}
