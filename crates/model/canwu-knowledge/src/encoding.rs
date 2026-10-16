//! The one canonical encoding of a knowledge record body, its digest, the
//! payload bytes a publication is charged, and the binary hashes of the
//! chunked ledger commitment.
//!
//! A body is encoded once, as compact JSON of its fields other than its
//! global ID, and streamed into blake3 without being kept. The same pass
//! counts the bytes of the payload's own encoding, which is exactly
//! `serde_json::to_vec(&payload).len()`: compact JSON writes a value nested
//! in an object with the same bytes it writes for the value alone.

use crate::{HolderEntry, KnowledgeOrigin, KnowledgeRecordBody, KnowledgeRecordDraft};
use crate::{KnowledgeSubject, KnowledgeSubjectTarget};
use canwu_core::{KnowledgeHolderRef, KnowledgeRecordKind, KnowledgeSchemaId};
use canwu_time::SimTime;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::cell::Cell;
use std::fmt;
use std::io::{self, Write};

const BODY_DOMAIN: &[u8] = b"canwu.knowledge.body.v1";
const ENTRY_DOMAIN: &[u8] = b"canwu.knowledge.entry.v1";
const CHUNK_DOMAIN: &[u8] = b"canwu.knowledge.chunk.v1";
const HOLDER_DOMAIN: &[u8] = b"canwu.knowledge.holder.v1";
const HOLDER_KEY_DOMAIN: &[u8] = b"canwu.knowledge.holder-key.v1";
const LEDGER_DOMAIN: &[u8] = b"canwu.knowledge.ledger.v1";
const KIND_KEY_DOMAIN: &[u8] = b"canwu.knowledge.kind-key.v1";
const SUBJECT_KEY_DOMAIN: &[u8] = b"canwu.knowledge.subject-key.v1";
const WRITER_BUFFER_BYTES: usize = 4_096;

/// A knowledge body or payload that could not be encoded as JSON.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeEncodingError {
    message: String,
}

impl KnowledgeEncodingError {
    fn new(error: &serde_json::Error) -> Self {
        Self {
            message: format!("knowledge record encoding failed: {error}"),
        }
    }

    /// What failed.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for KnowledgeEncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for KnowledgeEncodingError {}

/// The blake3 digest of one body's canonical encoding: the compact JSON of
/// its schema, subjects, payload, times, confidence, and origin, under the
/// domain `canwu.knowledge.body.v1`. The global ID is not part of it; the
/// commitment binds the ID beside the digest in each holder entry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KnowledgeBodyDigest([u8; 32]);

impl KnowledgeBodyDigest {
    /// The digest of a stored body.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeEncodingError`] when the body cannot be encoded.
    pub fn of(body: &KnowledgeRecordBody) -> Result<Self, KnowledgeEncodingError> {
        Ok(encode_body(&BodyParts::of_body(body))?.0)
    }

    /// The digest of a stored body and the payload bytes it was charged,
    /// from one encoding.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeEncodingError`] when the body cannot be encoded.
    pub fn with_payload_bytes(
        body: &KnowledgeRecordBody,
    ) -> Result<(Self, usize), KnowledgeEncodingError> {
        encode_body(&BodyParts::of_body(body))
    }

    /// The digest of the body `draft` becomes when it is learned at
    /// `learned_at`, and the bytes of its payload, from one encoding. The
    /// digest equals [`Self::of`] of that body whatever ID it is given.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeEncodingError`] when the draft cannot be encoded.
    pub fn of_draft(
        draft: &KnowledgeRecordDraft,
        learned_at: SimTime,
    ) -> Result<(Self, usize), KnowledgeEncodingError> {
        encode_body(&BodyParts {
            schema: &draft.schema,
            subjects: &draft.subjects,
            payload: &draft.payload,
            as_of: draft.as_of,
            learned_at,
            confidence_per_mille: draft.confidence_per_mille,
            origin: &draft.origin,
        })
    }

    /// A placeholder for bodies staged only to be read, never committed.
    pub(crate) const fn unhashed() -> Self {
        Self([0; 32])
    }

    /// The 32 digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The digest in lowercase hexadecimal.
    #[must_use]
    pub fn to_hex(&self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl Serialize for KnowledgeBodyDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl KnowledgeRecordDraft {
    /// The bytes a publication of this draft is charged against the
    /// boundary's payload budget: `serde_json::to_vec(&self.payload).len()`,
    /// counted without building the encoding. The engine charges exactly
    /// this number.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeEncodingError`] when the payload cannot be encoded.
    pub fn payload_bytes(&self) -> Result<usize, KnowledgeEncodingError> {
        let mut counter = ByteCounter(0);
        serde_json::to_writer(&mut counter, &self.payload)
            .map_err(|error| KnowledgeEncodingError::new(&error))?;
        Ok(usize::try_from(counter.0).unwrap_or(usize::MAX))
    }
}

impl KnowledgeRecordBody {
    /// The bytes the publication of this body was charged: the compact JSON
    /// length of its payload, as [`KnowledgeRecordDraft::payload_bytes`].
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeEncodingError`] when the payload cannot be encoded.
    pub fn payload_bytes(&self) -> Result<usize, KnowledgeEncodingError> {
        let mut counter = ByteCounter(0);
        serde_json::to_writer(&mut counter, &self.payload)
            .map_err(|error| KnowledgeEncodingError::new(&error))?;
        Ok(usize::try_from(counter.0).unwrap_or(usize::MAX))
    }
}

struct BodyParts<'a> {
    schema: &'a KnowledgeSchemaId,
    subjects: &'a [KnowledgeSubject],
    payload: &'a Value,
    as_of: Option<SimTime>,
    learned_at: SimTime,
    confidence_per_mille: u16,
    origin: &'a KnowledgeOrigin,
}

impl<'a> BodyParts<'a> {
    fn of_body(body: &'a KnowledgeRecordBody) -> Self {
        Self {
            schema: &body.schema,
            subjects: &body.subjects,
            payload: &body.payload,
            as_of: body.as_of,
            learned_at: body.learned_at,
            confidence_per_mille: body.confidence_per_mille,
            origin: &body.origin,
        }
    }
}

/// The payload, serialized as itself, recording how many bytes its
/// encoding took: the writer's count after it less the count before it.
/// `serde_json` writes an object field's key and colon before it serializes
/// the value and nothing after it, so the difference is the value's bytes.
struct CountedPayload<'a> {
    value: &'a Value,
    written: &'a Cell<u64>,
    bytes: &'a Cell<u64>,
}

impl Serialize for CountedPayload<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let start = self.written.get();
        let result = self.value.serialize(serializer);
        self.bytes.set(self.written.get().saturating_sub(start));
        result
    }
}

#[derive(Serialize)]
struct BodyMaterial<'a> {
    schema: &'a KnowledgeSchemaId,
    subjects: &'a [KnowledgeSubject],
    payload: CountedPayload<'a>,
    as_of: Option<SimTime>,
    learned_at: SimTime,
    confidence_per_mille: u16,
    origin: &'a KnowledgeOrigin,
}

fn encode_body(
    parts: &BodyParts<'_>,
) -> Result<(KnowledgeBodyDigest, usize), KnowledgeEncodingError> {
    let written = Cell::new(0);
    let bytes = Cell::new(0);
    let mut writer = DigestWriter::new(BODY_DOMAIN, Some(&written));
    serde_json::to_writer(
        &mut writer,
        &BodyMaterial {
            schema: parts.schema,
            subjects: parts.subjects,
            payload: CountedPayload {
                value: parts.payload,
                written: &written,
                bytes: &bytes,
            },
            as_of: parts.as_of,
            learned_at: parts.learned_at,
            confidence_per_mille: parts.confidence_per_mille,
            origin: parts.origin,
        },
    )
    .map_err(|error| KnowledgeEncodingError::new(&error))?;
    Ok((
        KnowledgeBodyDigest(writer.finish()),
        usize::try_from(bytes.get()).unwrap_or(usize::MAX),
    ))
}

/// Streams JSON into blake3 through a stack buffer, optionally counting
/// every byte it is given.
struct DigestWriter<'a> {
    hasher: blake3::Hasher,
    buffer: [u8; WRITER_BUFFER_BYTES],
    len: usize,
    written: Option<&'a Cell<u64>>,
}

impl<'a> DigestWriter<'a> {
    fn new(domain: &[u8], written: Option<&'a Cell<u64>>) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain);
        hasher.update(&[0]);
        Self {
            hasher,
            buffer: [0; WRITER_BUFFER_BYTES],
            len: 0,
            written,
        }
    }

    fn finish(mut self) -> [u8; 32] {
        self.hasher.update(&self.buffer[..self.len]);
        *self.hasher.finalize().as_bytes()
    }
}

impl Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(written) = self.written {
            written.set(written.get().saturating_add(bytes.len() as u64));
        }
        if self.len + bytes.len() > WRITER_BUFFER_BYTES {
            self.hasher.update(&self.buffer[..self.len]);
            self.len = 0;
        }
        if bytes.len() >= WRITER_BUFFER_BYTES {
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

struct ByteCounter(u64);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len() as u64);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A 16-byte key of the current-record index: two domain-separated
/// SipHash-1-3 digests of a schema kind or of one subject. Keys live only in
/// memory, are rebuilt on load, and never enter a commitment, so they need
/// to be deterministic but not cryptographic: the index keeps with each key
/// the exact kind or subject first filed under it, and a ledger in which two
/// values ever shared a key compares every candidate exactly, so a collision
/// never changes what a lookup returns.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct IndexKey([u64; 2]);

#[cfg(test)]
thread_local! {
    /// While set, every kind and every subject gets one key, so tests can
    /// drive the index through key collisions.
    pub(crate) static COLLIDING_KEYS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Hashes the same bytes under two seeds, written in a fixed byte order so
/// the key is the same on every platform.
struct KeyHasher([std::hash::DefaultHasher; 2]);

impl KeyHasher {
    fn new(domain: &[u8]) -> Self {
        use std::hash::Hasher as _;
        let mut halves = [
            std::hash::DefaultHasher::new(),
            std::hash::DefaultHasher::new(),
        ];
        for (seed, half) in halves.iter_mut().enumerate() {
            half.write(&[u8::try_from(seed).unwrap_or(u8::MAX)]);
            half.write(domain);
        }
        Self(halves)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        use std::hash::Hasher as _;
        for half in &mut self.0 {
            half.write(bytes);
        }
    }

    fn text(&mut self, text: &str) {
        self.bytes(&(text.len() as u64).to_le_bytes());
        self.bytes(text.as_bytes());
    }

    fn finish(&self) -> IndexKey {
        use std::hash::Hasher as _;
        #[cfg(test)]
        if COLLIDING_KEYS.get() {
            return IndexKey([0, 0]);
        }
        IndexKey([self.0[0].finish(), self.0[1].finish()])
    }
}

impl Write for KeyHasher {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn kind_key(kind: &KnowledgeRecordKind) -> IndexKey {
    let mut hasher = KeyHasher::new(KIND_KEY_DOMAIN);
    hasher.text(&kind.namespace);
    hasher.text(&kind.name);
    hasher.finish()
}

pub(crate) fn subject_key(subject: &KnowledgeSubject) -> IndexKey {
    let mut hasher = KeyHasher::new(SUBJECT_KEY_DOMAIN);
    hasher.text(&subject.role);
    match &subject.target {
        KnowledgeSubjectTarget::Event(event) => {
            hasher.bytes(b"e");
            hasher.bytes(&event.get().to_le_bytes());
        }
        KnowledgeSubjectTarget::DomainRecord(reference) => {
            hasher.bytes(b"d");
            hasher.text(&reference.kind.namespace);
            hasher.text(&reference.kind.name);
            hasher.text(&reference.id);
        }
        KnowledgeSubjectTarget::Entity(entity) => {
            // Entity references have many shapes; their canonical JSON tells
            // them apart. Encoding one cannot fail, and a failure would only
            // make the key coarser, never a lookup wrong.
            hasher.bytes(b"n");
            let encoded = serde_json::to_writer(&mut hasher, entity).is_ok();
            hasher.bytes(&[u8::from(encoded)]);
        }
    }
    hasher.finish()
}

/// The digest that stands for a holder in the ledger root.
pub(crate) fn holder_key(holder: &KnowledgeHolderRef) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(HOLDER_KEY_DOMAIN);
    hasher.update(&[0]);
    let encoded = serde_json::to_writer(HasherWriter(&mut hasher), holder).is_ok();
    hasher.update(&[u8::from(encoded)]);
    *hasher.finalize().as_bytes()
}

/// Feeds a short encoding straight into a hasher, without the stack buffer
/// a body's encoding streams through.
struct HasherWriter<'a>(&'a mut blake3::Hasher);

impl Write for HasherWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The leaf hash of one holder entry: its local ID, global ID, the digest of
/// its body, and its relation lists, in fixed-width binary.
pub(crate) fn leaf_hash(entry: &HolderEntry, digest: &KnowledgeBodyDigest) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ENTRY_DOMAIN);
    hasher.update(&[0]);
    hasher.update(&entry.local.get().to_le_bytes());
    hasher.update(&entry.record.get().to_le_bytes());
    hasher.update(digest.as_bytes());
    for list in [&entry.supersedes, &entry.contradicts] {
        hasher.update(&(list.len() as u64).to_le_bytes());
        for id in list {
            hasher.update(&id.get().to_le_bytes());
        }
    }
    *hasher.finalize().as_bytes()
}

/// The root of one chunk: blake3 over its leaf hashes in local order.
pub(crate) fn chunk_root<'a>(leaves: impl Iterator<Item = &'a [u8; 32]>) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CHUNK_DOMAIN);
    hasher.update(&[0]);
    for leaf in leaves {
        hasher.update(leaf);
    }
    *hasher.finalize().as_bytes()
}

/// The root of one holder: its next local ID and each non-empty chunk's
/// index and root, in chunk order. An empty holder's root binds its next
/// local ID alone.
pub(crate) fn holder_root<'a>(
    next_local: u64,
    chunks: impl Iterator<Item = (&'a u64, &'a [u8; 32])>,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(HOLDER_DOMAIN);
    hasher.update(&[0]);
    hasher.update(&next_local.to_le_bytes());
    for (index, root) in chunks {
        hasher.update(&index.to_le_bytes());
        hasher.update(root);
    }
    *hasher.finalize().as_bytes()
}

/// The ledger root: blake3 over each holder's key digest and root, in holder
/// order.
pub(crate) fn ledger_root<'a>(holders: impl Iterator<Item = (&'a [u8; 32], [u8; 32])>) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(LEDGER_DOMAIN);
    hasher.update(&[0]);
    for (key, root) in holders {
        hasher.update(key);
        hasher.update(&root);
    }
    *hasher.finalize().as_bytes()
}
