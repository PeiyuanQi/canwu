//! The holder projection root of a knowledge read cut, kept as a running
//! hash.
//!
//! The root is blake3, under the domain `canwu.knowledge.holder-projection.v1`
//! and a zero byte, of the compact JSON
//! `{"holder":<holder>,"full_history_views":[<view>,...]}`: every entry the
//! holder holds, in global ID order, as the holder-relative
//! [`crate::KnowledgeRecordView`] a page returns. This is the material 0.22.0
//! hashed from scratch on every read; the bytes are unchanged, so every read
//! cut, cursor, and record that stores one keeps its value.
//!
//! blake3 hashes a stream, so the material's prefix (everything before the
//! closing `]}`) can be kept as a hasher state and extended by one view when
//! an entry with a higher global ID than every held one arrives. Finalizing a
//! copy of that state with `]}` gives the root without reading an entry. A
//! change anywhere else (an entry that leaves, an entry admitted below the
//! highest global ID, a relation that comes to resolve) changes the bytes
//! from that entry on, so the holder ledger keeps a copy of the state every
//! [`PROJECTION_CHECKPOINT_VIEWS`] views and re-hashes from the last one
//! before the change.

use crate::{HolderEntry, KnowledgeRecordBody, KnowledgeSubject};
use canwu_core::{
    HolderKnowledgeRecordId, KnowledgeHolderRef, KnowledgeRecordId, KnowledgeSchemaId,
};
use canwu_time::SimTime;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::io::{self, Write};

const PROJECTION_DOMAIN: &[u8] = b"canwu.knowledge.holder-projection.v1";

/// The stack buffer a view's encoding streams through into the hash.
const STREAM_BUFFER_BYTES: usize = 4_096;

/// Views between two kept copies of a holder's projection hash state. Tests
/// use a short interval so that generated ledgers of a few hundred entries
/// cross many checkpoints.
pub(crate) const PROJECTION_CHECKPOINT_VIEWS: u64 = if cfg!(test) { 4 } else { 256 };

/// The projection material's hash state after some prefix of a holder's
/// entries, in global ID order.
#[derive(Clone)]
pub(crate) struct ProjectionState {
    hasher: blake3::Hasher,
    /// Views absorbed.
    views: u64,
    /// The global ID of the last view absorbed.
    last: Option<KnowledgeRecordId>,
    /// Whether some part could not be encoded; the root is then refused, as
    /// 0.22.0 refused it with an encoding error.
    encoding_failed: bool,
}

impl ProjectionState {
    /// The state before any view: the domain, the holder, and the opening of
    /// the view list.
    pub(crate) fn start(holder: &KnowledgeHolderRef) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(PROJECTION_DOMAIN);
        hasher.update(&[0]);
        hasher.update(br#"{"holder":"#);
        let mut buffer = Vec::new();
        let encoding_failed = serde_json::to_writer(&mut buffer, holder).is_err();
        hasher.update(&buffer);
        hasher.update(br#","full_history_views":["#);
        Self {
            hasher,
            views: 0,
            last: None,
            encoding_failed,
        }
    }

    /// Absorbs the view of the holder's next entry in global ID order.
    /// `local_of` names the holder's local ID of a global ID it holds. The
    /// view's encoding streams into the hash through a stack buffer, so a
    /// view allocates nothing in proportion to its payload.
    pub(crate) fn absorb(
        &mut self,
        holder: &KnowledgeHolderRef,
        entry: &HolderEntry,
        body: &KnowledgeRecordBody,
        local_of: &dyn Fn(&KnowledgeRecordId) -> Option<HolderKnowledgeRecordId>,
    ) {
        if self.views > 0 {
            self.hasher.update(b",");
        }
        let view = ViewMaterial {
            id: entry.local,
            holder,
            schema: &body.schema,
            subjects: &body.subjects,
            payload: &body.payload,
            as_of: body.as_of,
            learned_at: body.learned_at,
            confidence_per_mille: body.confidence_per_mille,
            supersedes: Locals {
                ids: &entry.supersedes,
                local_of,
            },
            contradicts: Locals {
                ids: &entry.contradicts,
                local_of,
            },
        };
        let mut stream = HashStream {
            hasher: &mut self.hasher,
            buffer: [0; STREAM_BUFFER_BYTES],
            len: 0,
        };
        self.encoding_failed |= serde_json::to_writer(&mut stream, &view).is_err();
        stream.finish();
        self.views += 1;
        self.last = Some(entry.record);
        crate::visits::view_hashed();
    }

    /// The root: a copy of the state finalized with the closing `]}`, or
    /// `None` when some part could not be encoded.
    pub(crate) fn root(&self) -> Option<String> {
        if self.encoding_failed {
            return None;
        }
        let mut hasher = self.hasher.clone();
        hasher.update(b"]}");
        Some(hasher.finalize().to_hex().to_string())
    }

    pub(crate) const fn views(&self) -> u64 {
        self.views
    }

    pub(crate) const fn last(&self) -> Option<KnowledgeRecordId> {
        self.last
    }
}

/// Streams bytes into a hasher through a stack buffer, as the body digest's
/// writer does, so the encoding is never built on the heap.
struct HashStream<'a> {
    hasher: &'a mut blake3::Hasher,
    buffer: [u8; STREAM_BUFFER_BYTES],
    len: usize,
}

impl HashStream<'_> {
    /// Hashes what is still buffered.
    fn finish(self) {
        self.hasher.update(&self.buffer[..self.len]);
    }
}

impl Write for HashStream<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.len + bytes.len() > STREAM_BUFFER_BYTES {
            self.hasher.update(&self.buffer[..self.len]);
            self.len = 0;
        }
        if bytes.len() >= STREAM_BUFFER_BYTES {
            self.hasher.update(bytes);
        } else {
            self.buffer[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A [`crate::KnowledgeRecordView`] by reference: the same fields, in the
/// same order, with the same encoding, without cloning the body.
#[derive(Serialize)]
struct ViewMaterial<'a> {
    id: HolderKnowledgeRecordId,
    holder: &'a KnowledgeHolderRef,
    schema: &'a KnowledgeSchemaId,
    subjects: &'a [KnowledgeSubject],
    payload: &'a Value,
    as_of: Option<SimTime>,
    learned_at: SimTime,
    confidence_per_mille: u16,
    supersedes: Locals<'a>,
    contradicts: Locals<'a>,
}

/// A relation list as the holder's local IDs, leaving out an ID the holder
/// does not hold, as [`crate::to_view`] does.
struct Locals<'a> {
    ids: &'a [KnowledgeRecordId],
    local_of: &'a dyn Fn(&KnowledgeRecordId) -> Option<HolderKnowledgeRecordId>,
}

impl Serialize for Locals<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.ids.iter().filter_map(|id| (self.local_of)(id)))
    }
}
