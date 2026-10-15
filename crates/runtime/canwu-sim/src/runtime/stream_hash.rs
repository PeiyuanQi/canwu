//! A BLAKE3 hash of an append-only byte stream whose state can be exported
//! and resumed.
//!
//! The command, attempt, event, ingress, and draw roots of the commitment are
//! `blake3::hash` of one growing byte stream each. `blake3::Hasher` cannot
//! export its state, so a run restored without the stream's prefix could not
//! extend it. This hasher keeps the same tree state the BLAKE3 reference
//! implementation keeps (the chaining values of completed subtrees and the
//! bytes of the current chunk) in a form that can be written out and read
//! back, and computes exactly `blake3::hash` of the bytes fed. It is built on
//! the `blake3::hazmat` subtree functions, following section 5.1.2 of the
//! BLAKE3 paper.

use super::{CanwuError, ErrorCode};
use blake3::hazmat::{
    ChainingValue, HasherExt, Mode, merge_subtrees_non_root, merge_subtrees_root,
};
use blake3::{CHUNK_LEN, Hasher};

/// The deepest chaining-value stack a 64-bit input length can need.
const MAX_STACK_DEPTH: usize = 54;
const CHUNK_BYTES: u64 = CHUNK_LEN as u64;
/// The longest stream a frontier may resume. It is far beyond any journal and
/// leaves more than 2^63 bytes of room, so a resumed stream cannot be one
/// update away from overflowing its length.
const MAX_RESUMED_LEN: u64 = 1 << 62;

fn length_overflow() -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidSnapshot,
        "journal stream length is not representable",
    )
}

/// The resumable state of one hashed stream.
///
/// It holds no heap allocation, so a copy for a rollback checkpoint costs a
/// fixed `memcpy`, as copying a `blake3::Hasher` did.
#[derive(Clone)]
pub(super) struct ResumableStreamHash {
    /// Bytes fed so far.
    len: u64,
    /// Chaining values of the completed subtrees, largest (leftmost) first.
    stack: [ChainingValue; MAX_STACK_DEPTH],
    stack_len: usize,
    /// The bytes of the current chunk. A chunk is finalized only when more
    /// bytes follow it, so once anything has been fed it is never empty.
    chunk: [u8; CHUNK_LEN],
    chunk_len: usize,
}

/// The exported state of a [`ResumableStreamHash`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StreamFrontier {
    pub(super) len: u64,
    pub(super) chaining_values: Vec<ChainingValue>,
    pub(super) tail: Vec<u8>,
}

impl ResumableStreamHash {
    pub(super) const fn new() -> Self {
        Self {
            len: 0,
            stack: [[0; 32]; MAX_STACK_DEPTH],
            stack_len: 0,
            chunk: [0; CHUNK_LEN],
            chunk_len: 0,
        }
    }

    /// Feeds bytes to the stream, refusing input whose total length would
    /// not fit in 64 bits before any of it is hashed.
    pub(super) fn update(&mut self, mut input: &[u8]) -> Result<(), CanwuError> {
        u64::try_from(input.len())
            .ok()
            .and_then(|added| self.len.checked_add(added))
            .ok_or_else(length_overflow)?;
        while !input.is_empty() {
            if self.chunk_len == CHUNK_LEN {
                // More bytes follow a full chunk, so it is not the root and
                // can be finalized and merged.
                let offset = self.len - CHUNK_BYTES;
                let chaining_value = Hasher::new()
                    .set_input_offset(offset)
                    .update(&self.chunk)
                    .finalize_non_root();
                self.push_subtree(chaining_value, offset / CHUNK_BYTES + 1, 1);
                self.chunk_len = 0;
            }
            if self.chunk_len == 0 && input.len() > CHUNK_LEN {
                // Whole chunks with more input after them are not the root.
                // Hash the largest aligned power-of-two run of them in one
                // call, so BLAKE3 can compress several chunks at once.
                let subtree_chunks = self.aligned_subtree_chunks((input.len() - 1) / CHUNK_LEN);
                let subtree_len = subtree_chunks * CHUNK_LEN;
                let chaining_value = Hasher::new()
                    .set_input_offset(self.len)
                    .update(&input[..subtree_len])
                    .finalize_non_root();
                // `subtree_len` is at most the input length, checked above.
                self.len += u64::try_from(subtree_len).unwrap_or(u64::MAX);
                self.push_subtree(
                    chaining_value,
                    self.len / CHUNK_BYTES,
                    u64::try_from(subtree_chunks).unwrap_or(u64::MAX),
                );
                input = &input[subtree_len..];
                continue;
            }
            let start = self.chunk_len;
            let take = (CHUNK_LEN - start).min(input.len());
            self.chunk[start..start + take].copy_from_slice(&input[..take]);
            self.chunk_len += take;
            // `take` is at most `CHUNK_LEN`, and the sum was checked above.
            self.len += CHUNK_BYTES.min(u64::try_from(take).unwrap_or(CHUNK_BYTES));
            input = &input[take..];
        }
        Ok(())
    }

    /// The number of whole chunks, a power of two and at most `available`,
    /// that the next subtree can cover. A subtree must start at a multiple
    /// of its own size, which BLAKE3's tree shape requires.
    fn aligned_subtree_chunks(&self, available: usize) -> usize {
        let largest = 1_usize << available.ilog2();
        let completed = self.len / CHUNK_BYTES;
        if completed == 0 {
            return largest;
        }
        let alignment = usize::try_from(1_u64 << completed.trailing_zeros()).unwrap_or(usize::MAX);
        largest.min(alignment)
    }

    /// Adds the chaining value of an aligned subtree of `subtree_chunks`
    /// chunks that ends after chunk number `completed_chunks` (counting from
    /// one), and merges every larger subtree it completes, as the BLAKE3
    /// reference implementation does one chunk at a time.
    fn push_subtree(
        &mut self,
        mut chaining_value: ChainingValue,
        completed_chunks: u64,
        subtree_chunks: u64,
    ) {
        let mut level = completed_chunks / subtree_chunks;
        while level & 1 == 0 {
            self.stack_len -= 1;
            let left = self.stack[self.stack_len];
            chaining_value = merge_subtrees_non_root(&left, &chaining_value, Mode::Hash);
            level >>= 1;
        }
        self.stack[self.stack_len] = chaining_value;
        self.stack_len += 1;
    }

    /// `blake3::hash` of every byte fed so far.
    pub(super) fn finalize(&self) -> blake3::Hash {
        let chunk = &self.chunk[..self.chunk_len];
        if self.stack_len == 0 {
            // A single chunk, possibly empty, is the root itself.
            return Hasher::new().update(chunk).finalize();
        }
        let offset = self.len - u64::try_from(self.chunk_len).unwrap_or(CHUNK_BYTES);
        let mut right = Hasher::new()
            .set_input_offset(offset)
            .update(chunk)
            .finalize_non_root();
        for index in (1..self.stack_len).rev() {
            right = merge_subtrees_non_root(&self.stack[index], &right, Mode::Hash);
        }
        merge_subtrees_root(&self.stack[0], &right, Mode::Hash)
    }

    /// Exports the state.
    pub(super) fn frontier(&self) -> StreamFrontier {
        StreamFrontier {
            len: self.len,
            chaining_values: self.stack[..self.stack_len].to_vec(),
            tail: self.chunk[..self.chunk_len].to_vec(),
        }
    }

    /// Resumes an exported state, refusing any shape the hasher could not
    /// have produced before a hazmat call could assert on it.
    pub(super) fn from_frontier(frontier: &StreamFrontier) -> Result<Self, CanwuError> {
        let invalid = |message: &str| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("journal frontier is invalid: {message}"),
            )
        };
        let tail_len = u64::try_from(frontier.tail.len())
            .map_err(|_| invalid("its tail length is not representable"))?;
        if frontier.len > MAX_RESUMED_LEN {
            return Err(invalid("its length exceeds 2^62 bytes"));
        }
        if frontier.len == 0 {
            if !frontier.tail.is_empty() || !frontier.chaining_values.is_empty() {
                return Err(invalid("an empty stream has a tail or chaining values"));
            }
            return Ok(Self::new());
        }
        if tail_len == 0 || tail_len > CHUNK_BYTES || tail_len > frontier.len {
            return Err(invalid("its tail must hold one to 1,024 bytes"));
        }
        let completed = frontier.len - tail_len;
        if !completed.is_multiple_of(CHUNK_BYTES) {
            return Err(invalid("its tail does not start at a chunk boundary"));
        }
        let expected_depth = (completed / CHUNK_BYTES).count_ones() as usize;
        if frontier.chaining_values.len() != expected_depth || expected_depth > MAX_STACK_DEPTH {
            return Err(invalid(
                "its chaining values do not match the completed chunk count",
            ));
        }
        let mut resumed = Self::new();
        resumed.len = frontier.len;
        resumed.stack[..expected_depth].copy_from_slice(&frontier.chaining_values);
        resumed.stack_len = expected_depth;
        resumed.chunk[..frontier.tail.len()].copy_from_slice(&frontier.tail);
        resumed.chunk_len = frontier.tail.len();
        Ok(resumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic bytes without a random-number dependency.
    fn stream(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect()
    }

    fn lengths() -> Vec<usize> {
        let mut lengths = vec![0, 1, 63, 64, 65, 1023, 1024, 1025, 2047, 2048, 2049];
        for chunks in [3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33, 47, 48] {
            for delta in [-1_isize, 0, 1] {
                let len = (chunks * CHUNK_LEN).checked_add_signed(delta).unwrap_or(0);
                lengths.push(len);
            }
        }
        lengths
    }

    #[test]
    fn every_split_and_resume_point_reproduces_blake3() {
        for (case, len) in lengths().into_iter().enumerate() {
            let bytes = stream(len, case as u64);
            let expected = blake3::hash(&bytes);
            let mut whole = ResumableStreamHash::new();
            whole.update(&bytes).unwrap();
            assert_eq!(whole.finalize(), expected, "one update of {len} bytes");

            // Feed in irregular pieces, exporting and resuming at every one.
            let mut hasher = ResumableStreamHash::new();
            let mut fed = 0;
            let mut piece = 1 + case % 97;
            while fed < len {
                let end = (fed + piece).min(len);
                hasher.update(&bytes[fed..end]).unwrap();
                fed = end;
                let frontier = hasher.frontier();
                hasher = ResumableStreamHash::from_frontier(&frontier)
                    .unwrap_or_else(|error| panic!("{len} at {fed}: {}", error.message));
                assert_eq!(hasher.frontier(), frontier);
                assert_eq!(
                    hasher.finalize(),
                    blake3::hash(&bytes[..fed]),
                    "{len} bytes resumed at {fed}"
                );
                piece = piece * 7 % 9_973 + 1;
            }
            assert_eq!(hasher.finalize(), expected, "{len} bytes in pieces");
        }
    }

    #[test]
    fn upstream_test_vector_inputs_hash_alike() {
        // The official BLAKE3 test vectors hash `i % 251` for these lengths.
        for len in [
            0, 1, 1023, 1024, 1025, 2048, 2049, 3072, 3073, 4096, 4097, 5120, 5121, 6144, 6145,
            7168, 7169, 8192, 8193, 16384, 31744, 102_400,
        ] {
            let bytes: Vec<u8> = (0..len)
                .map(|index: usize| u8::try_from(index % 251).expect("below 251"))
                .collect();
            let mut hasher = ResumableStreamHash::new();
            for piece in bytes.chunks(333) {
                hasher.update(piece).unwrap();
            }
            assert_eq!(hasher.finalize(), blake3::hash(&bytes), "{len} bytes");
            let mut whole = ResumableStreamHash::new();
            whole.update(&bytes).unwrap();
            assert_eq!(
                whole.finalize(),
                blake3::hash(&bytes),
                "{len} bytes at once"
            );
        }
        let one_kib: Vec<u8> = (0..1024_usize)
            .map(|index| u8::try_from(index % 251).expect("below 251"))
            .collect();
        let mut hasher = ResumableStreamHash::new();
        hasher.update(&one_kib).unwrap();
        assert_eq!(
            hasher.finalize().to_hex().as_str(),
            "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"
        );
    }

    #[test]
    fn a_frontier_no_stream_could_produce_is_refused() {
        let mut hasher = ResumableStreamHash::new();
        hasher.update(&stream(3 * CHUNK_LEN + 5, 7)).unwrap();
        let good = hasher.frontier();
        assert!(ResumableStreamHash::from_frontier(&good).is_ok());
        let mut cases = Vec::new();
        let mut empty_tail = good.clone();
        empty_tail.tail.clear();
        cases.push(empty_tail);
        let mut long_tail = good.clone();
        long_tail.tail = vec![0; CHUNK_LEN + 1];
        long_tail.len += CHUNK_BYTES - 4;
        cases.push(long_tail);
        let mut misaligned = good.clone();
        misaligned.len += 1;
        cases.push(misaligned);
        let mut missing_value = good.clone();
        missing_value.chaining_values.pop();
        cases.push(missing_value);
        let mut extra_value = good.clone();
        extra_value.chaining_values.push([0; 32]);
        cases.push(extra_value);
        let mut empty_with_tail = good;
        empty_with_tail.len = 0;
        cases.push(empty_with_tail);
        // The longest stream whose shape is otherwise consistent: a 1,023-byte
        // tail after 2^54 - 1 completed chunks, one chaining value per set bit.
        let near_overflow = StreamFrontier {
            len: u64::MAX,
            chaining_values: vec![[0; 32]; 54],
            tail: vec![0; CHUNK_LEN - 1],
        };
        cases.push(near_overflow);
        for case in cases {
            let error = ResumableStreamHash::from_frontier(&case)
                .err()
                .expect("a malformed frontier is refused");
            assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        }
    }

    #[test]
    fn stream_length_is_capped_on_resume_and_checked_on_update() {
        // The longest resumable frontier: 2^52 - 1 completed chunks, one
        // chaining value per set bit, and a full tail.
        let longest = StreamFrontier {
            len: MAX_RESUMED_LEN,
            chaining_values: vec![[0; 32]; 52],
            tail: vec![0; CHUNK_LEN],
        };
        let mut hasher =
            ResumableStreamHash::from_frontier(&longest).expect("a 2^62-byte frontier is accepted");
        hasher.update(b",").unwrap();
        assert_eq!(hasher.frontier().len, MAX_RESUMED_LEN + 1);
        // One byte more: 2^52 completed chunks and a one-byte tail.
        let over = StreamFrontier {
            len: MAX_RESUMED_LEN + 1,
            chaining_values: vec![[0; 32]; 1],
            tail: vec![0; 1],
        };
        assert_eq!(
            ResumableStreamHash::from_frontier(&over)
                .err()
                .expect("a longer frontier is refused")
                .code,
            ErrorCode::InvalidSnapshot
        );

        // A length one byte short of the 64-bit limit refuses two more bytes
        // and keeps its state.
        let mut full = ResumableStreamHash::new();
        full.update(&[7; 5]).unwrap();
        full.len = u64::MAX - 1;
        let before = full.frontier();
        let error = full.update(&[0, 0]).expect_err("the length would overflow");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert_eq!(full.frontier(), before);
    }
}
