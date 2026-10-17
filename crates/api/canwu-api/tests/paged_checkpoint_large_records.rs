//! A paged checkpoint holds a domain record of any length up to
//! `MAX_PAGED_RECORD_BYTES`, not only one that fits one state page.
//!
//! A record longer than `STATE_PAGE_VALUE_CHUNK_BYTES` is split into
//! record-value chunk pages that its Patricia leaf names by page ID. A run
//! holding records of several megabytes writes and restores paged
//! checkpoints and their deltas with the checkpoint hash and commitment roots
//! of the flat checkpoint path, the retention closure reaches every chunk,
//! and a later delta carries only the chunks that changed. A chunk that does
//! not match its ID, a chunk that is missing, and leaf pages whose chunk
//! count, chunk lengths, or declared length are out of bounds are refused
//! without a panic.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, DomainRecord, DomainRecordClass,
    DomainRecordCommitmentRoots, DomainRecordDraft, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordMutation, DomainRecordPageRoots, DomainRecordRef, DomainRecordSchema, ErrorCode,
    EvidenceCursor, EvidenceJournalSegment, MAX_PAGED_RECORD_BYTES, MAX_STATE_PAGE_BYTES,
    PagedRestoreBudget, PersistentDomainRecordStore, PluginRegistrar, STATE_PAGE_CODEC,
    STATE_PAGE_VALUE_CHUNK_BYTES, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView,
    StateKey, StatePageBlob, StatePageProvider, StatePageRetentionLedger, StatePageStore,
    StateVisibility, SystemCadence, canonical_byte_hash,
};
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const ARCHIVE: &str = "fixture-archive";
const PRIMARY_DOMAIN: &str = "canwu.format8.domain-record.primary.v1";
/// The large record's payload: four full chunks and part of a fifth.
const LARGE_PAYLOAD_BYTES: usize = 4 * STATE_PAGE_VALUE_CHUNK_BYTES + 300_000;
/// Held inline in its leaf, just under one chunk.
const MEDIUM_PAYLOAD_BYTES: usize = 1_000_000;

fn kind() -> DomainRecordKind {
    DomainRecordKind::new(ARCHIVE, "volume")
}

fn volume(id: &str) -> DomainRecordRef {
    DomainRecordRef {
        kind: kind(),
        id: id.to_owned(),
    }
}

fn archive_state() -> StateKey {
    DomainRecordSchema::new(kind(), DomainRecordClass::Record).state_key()
}

fn large_payload(boundary: u64) -> serde_json::Value {
    // Numbered text, so no two chunks share content. Each later boundary
    // appends to the end of the text, so the record's version near its start
    // and its tail change, and its middle does not.
    let mut text = String::with_capacity(LARGE_PAYLOAD_BYTES + 64);
    for ordinal in 0..LARGE_PAYLOAD_BYTES / 8 {
        write!(text, "{ordinal:07},").expect("writing to a String cannot fail");
    }
    for _ in 1..boundary {
        text.push_str("appended");
    }
    json!({ "text": text })
}

/// Creates the records at boundary 1, appends to the large one at boundary
/// 2, and posts to the small one afterwards.
fn keep_archive(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = context.boundary_id.get();
    let create = |id: &str, payload: serde_json::Value| BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Create {
            record: DomainRecordDraft::new(volume(id), payload),
        },
        summary: format!("Opened volume {id}"),
    };
    let update = |id: &str, payload: serde_json::Value| -> Result<_, CanwuError> {
        let current = view
            .domain_record(&volume(id))?
            .expect("volumes exist after the opening boundary")
            .version;
        Ok(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(volume(id), payload),
                expected_version: current,
            },
            summary: format!("Revised volume {id}"),
        })
    };
    let directives = match boundary {
        1 => vec![
            create("large", large_payload(1)),
            create(
                "medium",
                json!({ "text": "m".repeat(MEDIUM_PAYLOAD_BYTES) }),
            ),
            create("small", json!({ "entries": 0 })),
        ],
        2 => vec![update("large", large_payload(2))?],
        _ => vec![update("small", json!({ "entries": boundary }))?],
    };
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct ArchivePlugin;

impl SimulationPlugin for ArchivePlugin {
    fn name(&self) -> &'static str {
        ARCHIVE
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000c4a7c5"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar
            .register_record_schema(DomainRecordSchema::new(kind(), DomainRecordClass::Record))?;
        let mut contract = BoundarySystemContract::new(
            "keep-archive",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![archive_state()];
        contract.writes = vec![archive_state()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, keep_archive)
    }
}

/// A content-addressed page store, as a host backs it.
#[derive(Clone, Default)]
struct PageStore(RefCell<BTreeMap<String, StatePageBlob>>);

impl StatePageProvider for PageStore {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(self.0.borrow().get(page_id).cloned())
    }
}

impl StatePageStore for PageStore {
    fn store_state_page(
        &self,
        page: &StatePageBlob,
    ) -> Result<canwu_api::ArchiveStoreOutcome, CanwuError> {
        page.validate()?;
        let mut pages = self.0.borrow_mut();
        if let Some(existing) = pages.get(&page.page_id) {
            assert_eq!(existing, page, "one page ID must name one byte string");
            return Ok(canwu_api::ArchiveStoreOutcome::AlreadyPresent);
        }
        pages.insert(page.page_id.clone(), page.clone());
        Ok(canwu_api::ArchiveStoreOutcome::Stored)
    }
}

fn is_chunk_page(page: &StatePageBlob) -> bool {
    page.bytes.starts_with(b"\0cwp\x03")
}

fn daily(day: i64) -> BoundaryRequest {
    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day)).with_cadence(SystemCadence::Daily)
}

struct Host {
    canwu: Canwu,
    cursor: EvidenceCursor,
    segments: Vec<EvidenceJournalSegment>,
}

impl Host {
    fn settle(&mut self, day: i64) {
        self.canwu
            .settle_boundary(daily(day))
            .expect("archive boundary settles");
        self.segments.push(
            self.canwu
                .journal_segment_since(self.cursor)
                .expect("boundary evidence segment"),
        );
        self.cursor = self.canwu.evidence_cursor().expect("evidence cursor");
    }
}

fn commitment_roots(canwu: &Canwu) -> canwu_api::CommitmentRoots {
    canwu
        .checkpoint()
        .expect("checkpoint")
        .state
        .commitment_roots
        .expect("current commitment roots")
}

fn assert_same_run(restored: &Canwu, live: &Canwu) {
    assert_eq!(restored.checkpoint_hash(), live.checkpoint_hash());
    assert_eq!(commitment_roots(restored), commitment_roots(live));
    assert_eq!(
        restored
            .authoritative_state_hash()
            .expect("restored state hash"),
        live.authoritative_state_hash().expect("live state hash")
    );
    assert_eq!(restored.revision(), live.revision());
    for id in ["large", "medium", "small"] {
        assert_eq!(
            restored.domain_record(&volume(id)),
            live.domain_record(&volume(id))
        );
    }
}

fn restore(
    checkpoint: &canwu_api::PagedSimulationCheckpoint,
    store: &PageStore,
    segments: &[EvidenceJournalSegment],
) -> Result<Canwu, CanwuError> {
    Canwu::from_paged_checkpoint_and_journal_with_plugins(
        checkpoint,
        store,
        segments.to_vec(),
        &[&ArchivePlugin],
        PagedRestoreBudget::DEFAULT,
    )
}

#[test]
fn records_longer_than_a_page_round_trip_through_paged_checkpoints_and_deltas() {
    assert_eq!(STATE_PAGE_CODEC, "raw-canonical-v2");
    let mut live = Host {
        canwu: Canwu::new_with_plugins(
            4747,
            Scenario::new(SimTime::EPOCH, Vec::new()),
            &[&ArchivePlugin],
        )
        .expect("archive run"),
        cursor: EvidenceCursor::default(),
        segments: Vec::new(),
    };
    live.settle(1);

    let store = PageStore::default();
    let first = live
        .canwu
        .prepare_paged_checkpoint(None, &store)
        .expect("a paged checkpoint holds records longer than one state page");
    assert!(
        first
            .delta
            .new_pages
            .iter()
            .all(|page| page.bytes.len() <= MAX_STATE_PAGE_BYTES)
    );
    let first_chunks = first
        .delta
        .new_pages
        .iter()
        .filter(|page| is_chunk_page(page))
        .count();
    // The large record spans five chunk pages; the compact checkpoint body
    // and the hot decision state are one chunk page each.
    assert_eq!(first_chunks, 7, "the large record spans five chunk pages");
    first
        .store_and_verify(&store)
        .expect("store the first paged checkpoint");

    // The retention closure walks from the root into every chunk page, so a
    // host's collector keeps them.
    let mut ledger = StatePageRetentionLedger::default();
    let handle = ledger
        .prepare(&first.delta, &store)
        .expect("prepare retention");
    ledger.verify(&handle, &store).expect("verify retention");
    let retained = &ledger.handles[&handle].page_ids;
    assert!(
        first
            .delta
            .new_pages
            .iter()
            .all(|page| retained.contains(&page.page_id))
    );

    // The paged restore equals the live run and the flat checkpoint path.
    let paged = restore(&first.checkpoint, &store, &live.segments)
        .expect("restore the first paged checkpoint");
    assert_same_run(&paged, &live.canwu);
    let flat = Canwu::from_checkpoint_journal_with_plugins(
        live.canwu
            .checkpoint_journal()
            .expect("flat checkpoint journal"),
        &[&ArchivePlugin],
    )
    .expect("restore the flat checkpoint");
    assert_same_run(&flat, &paged);

    // Appending to the large record changes the chunk holding its version
    // and its last chunk; the next delta reuses the three in between.
    live.settle(2);
    let second = live
        .canwu
        .prepare_paged_checkpoint(Some(&first.checkpoint), &store)
        .expect("prepare the second paged checkpoint");
    let second_chunks = second
        .delta
        .new_pages
        .iter()
        .filter(|page| is_chunk_page(page))
        .count();
    // Two of the record's chunks changed, and the body's chunk with the
    // checkpoint hash and counters; the hot decision state did not.
    assert_eq!(second_chunks, 3, "only the changed chunks are new");
    second
        .store_and_verify(&store)
        .expect("store the second paged checkpoint");
    let mut resumed = Host {
        canwu: restore(&second.checkpoint, &store, &live.segments)
            .expect("restore the second paged checkpoint"),
        cursor: live.cursor,
        segments: live.segments.clone(),
    };
    assert_same_run(&resumed.canwu, &live.canwu);

    // The restored run continues exactly as the live run does.
    live.settle(3);
    resumed.settle(3);
    assert_same_run(&resumed.canwu, &live.canwu);
    assert_eq!(resumed.segments, live.segments);
}

#[test]
fn tampered_or_missing_record_chunks_are_refused() {
    let mut live = Host {
        canwu: Canwu::new_with_plugins(
            4848,
            Scenario::new(SimTime::EPOCH, Vec::new()),
            &[&ArchivePlugin],
        )
        .expect("archive run"),
        cursor: EvidenceCursor::default(),
        segments: Vec::new(),
    };
    live.settle(1);
    let store = PageStore::default();
    let prepared = live
        .canwu
        .prepare_paged_checkpoint(None, &store)
        .expect("prepare the paged checkpoint");
    prepared.store_and_verify(&store).expect("store the pages");
    let chunk_ids = prepared
        .delta
        .new_pages
        .iter()
        .filter(|page| is_chunk_page(page))
        .map(|page| page.page_id.clone())
        .collect::<Vec<_>>();
    assert!(chunk_ids.len() > 1);

    // A chunk whose bytes no longer match its ID.
    let tampered = store.clone();
    tampered
        .0
        .borrow_mut()
        .get_mut(&chunk_ids[1])
        .expect("stored chunk")
        .bytes[100] ^= 1;
    let error = restore(&prepared.checkpoint, &tampered, &live.segments)
        .err()
        .expect("a tampered chunk is refused");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A provider that answers a chunk ID with a different, valid chunk.
    let swapped = store.clone();
    let other = swapped.0.borrow()[&chunk_ids[0]].clone();
    swapped.0.borrow_mut().insert(chunk_ids[1].clone(), other);
    let error = restore(&prepared.checkpoint, &swapped, &live.segments)
        .err()
        .expect("a chunk under another chunk's ID is refused");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A chunk the provider lost.
    let torn = store.clone();
    torn.0.borrow_mut().remove(&chunk_ids[1]);
    let error = restore(&prepared.checkpoint, &torn, &live.segments)
        .err()
        .expect("a missing chunk is refused");
    assert_eq!(error.code, ErrorCode::StatePageUnavailable);
}

/// Pages built by hand from the documented layout of state-page codec
/// `raw-canonical-v2`, every integer little-endian.
fn domain_bytes() -> Vec<u8> {
    let mut bytes = u16::try_from(PRIMARY_DOMAIN.len())
        .expect("short domain")
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(PRIMARY_DOMAIN.as_bytes());
    bytes
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex"))
        .collect()
}

fn length(value: usize) -> [u8; 8] {
    u64::try_from(value).expect("length").to_le_bytes()
}

/// One primary-index leaf with one entry for `reference`: `value_len`
/// followed by `value`, which holds the inline bytes or the chunk IDs.
fn leaf_page(reference: &DomainRecordRef, value_len: u64, value: &[u8]) -> Vec<u8> {
    let key = serde_json::to_vec(reference).expect("encode key");
    let mut bytes = b"\0cwp\x01".to_vec();
    bytes.extend(domain_bytes());
    bytes.extend(hex_bytes(&canonical_byte_hash(
        &format!("{PRIMARY_DOMAIN}.key"),
        &key,
    )));
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&length(32 + 8 + key.len() + 32));
    bytes.extend_from_slice(&length(key.len()));
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(&value_len.to_le_bytes());
    bytes.extend_from_slice(value);
    bytes
}

fn chunk_page(payload: &[u8]) -> StatePageBlob {
    let mut bytes = b"\0cwp\x03".to_vec();
    bytes.extend_from_slice(payload);
    StatePageBlob::new(bytes).expect("chunk page")
}

fn page_store<'a>(pages: impl IntoIterator<Item = &'a StatePageBlob>) -> PageStore {
    let store = PageStore::default();
    for page in pages {
        store
            .0
            .borrow_mut()
            .insert(page.page_id.clone(), page.clone());
    }
    store
}

fn record(reference: &DomainRecordRef, text: &str) -> DomainRecord {
    DomainRecord {
        reference: reference.clone(),
        owner: ARCHIVE.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: json!({ "text": text }),
        references: Vec::new(),
    }
}

/// Loads a one-leaf primary index whose root page is `root`.
fn load_hostile_leaf(root: Vec<u8>, pages: &[StatePageBlob]) -> CanwuError {
    let root = StatePageBlob::new(root).expect("root page");
    let store = page_store(pages.iter().chain([&root]));
    PersistentDomainRecordStore::from_state_pages(
        &hostile_roots(&root),
        &store,
        PagedRestoreBudget::DEFAULT,
    )
    .expect_err("the hostile page graph is refused")
}

/// The page roots of a one-leaf primary index whose root page is `root`.
fn hostile_roots(root: &StatePageBlob) -> DomainRecordPageRoots {
    let unused = "0".repeat(64);
    DomainRecordPageRoots {
        primary: Some(root.page_id.clone()),
        reverse_references: None,
        successor_of: None,
        predecessors_of: None,
        commitment_roots: DomainRecordCommitmentRoots {
            primary: root.page_id.clone(),
            reverse_references: unused.clone(),
            successor_of: unused.clone(),
            predecessors_of: unused,
        },
    }
}

/// A provider that counts how often each page is read.
struct CountingStore {
    store: PageStore,
    reads: RefCell<BTreeMap<String, usize>>,
}

impl StatePageProvider for CountingStore {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        *self
            .reads
            .borrow_mut()
            .entry(page_id.to_owned())
            .or_default() += 1;
        self.store.load_state_page(page_id)
    }
}

/// A leaf may name one chunk page many times, so a few distinct bytes can
/// describe a long value. The restore charges the length a leaf describes to
/// its budget before it reads a chunk or allocates the value, and refuses a
/// value that would pass the budget; at exactly the budget it goes on to the
/// usual checks.
#[test]
fn the_restore_budget_refuses_a_described_value_before_reading_it() {
    let reference = volume("amplified");
    let full = chunk_page(&vec![b'a'; STATE_PAGE_VALUE_CHUNK_BYTES]);
    let repeats = 8;
    let ids = std::iter::repeat_n(hex_bytes(&full.page_id), repeats)
        .flatten()
        .collect::<Vec<_>>();
    let described = u64::try_from(repeats * STATE_PAGE_VALUE_CHUNK_BYTES).expect("length");
    let root = StatePageBlob::new(leaf_page(&reference, described, &ids)).expect("root page");
    let provider = CountingStore {
        store: page_store([&full, &root]),
        reads: RefCell::default(),
    };

    let error = PersistentDomainRecordStore::from_state_pages(
        &hostile_roots(&root),
        &provider,
        PagedRestoreBudget::new(described - 1),
    )
    .expect_err("a value longer than the budget is refused");
    assert_eq!(error.code, ErrorCode::RestoreBudgetExceeded);
    assert_eq!(
        provider.reads.borrow().get(&full.page_id),
        None,
        "no chunk of the refused value was read"
    );

    // At exactly its length the value is assembled from eight reads of one
    // chunk, and then refused only because it is not a record.
    let error = PersistentDomainRecordStore::from_state_pages(
        &hostile_roots(&root),
        &provider,
        PagedRestoreBudget::new(described),
    )
    .expect_err("eight mebibytes of one byte are not a record");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert_eq!(provider.reads.borrow().get(&full.page_id), Some(&repeats));
}

#[test]
fn record_pages_load_only_in_their_canonical_bounded_form() {
    let chunk = u64::try_from(STATE_PAGE_VALUE_CHUNK_BYTES).expect("chunk length");

    // The documented layout is the engine's: hand-built pages of one inline
    // and one chunked record equal the pages the store emits, and load.
    for (id, text_len) in [("inline", 1_000), ("chunked", STATE_PAGE_VALUE_CHUNK_BYTES)] {
        let reference = volume(id);
        let record = record(&reference, &"c".repeat(text_len));
        let value = serde_json::to_vec(&record).expect("encode record");
        let store = PersistentDomainRecordStore::from_records(BTreeMap::from([(
            reference.clone(),
            record,
        )]))
        .expect("store");
        let (roots, pages) = store.state_pages().expect("state pages");
        let (chunks, leaf) = if value.len() <= STATE_PAGE_VALUE_CHUNK_BYTES {
            (
                Vec::new(),
                leaf_page(&reference, value.len() as u64, &value),
            )
        } else {
            let chunks = value
                .chunks(STATE_PAGE_VALUE_CHUNK_BYTES)
                .map(chunk_page)
                .collect::<Vec<_>>();
            assert_eq!(chunks.len(), 2);
            let ids = chunks
                .iter()
                .flat_map(|page| hex_bytes(&page.page_id))
                .collect::<Vec<_>>();
            (chunks, leaf_page(&reference, value.len() as u64, &ids))
        };
        let leaf = StatePageBlob::new(leaf).expect("leaf page");
        assert_eq!(roots.primary.as_ref(), Some(&leaf.page_id));
        let emitted = pages
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<BTreeSet<_>>();
        for page in chunks.iter().chain([&leaf]) {
            assert!(emitted.contains(&page.page_id));
        }
        let hand_built = page_store(
            pages
                .iter()
                .filter(|page| page.page_id != leaf.page_id && !is_chunk_page(page))
                .chain(chunks.iter())
                .chain([&leaf]),
        );
        let loaded = PersistentDomainRecordStore::from_state_pages(
            &roots,
            &hand_built,
            PagedRestoreBudget::DEFAULT,
        )
        .expect("hand-built canonical pages load");
        assert_eq!(loaded.materialize(), store.materialize());
    }

    let reference = volume("hostile");
    // A declared length above the paged record ceiling is refused before any
    // of its chunks is read, even with a chunk ID for every MiB it claims.
    let over_ceiling = MAX_PAGED_RECORD_BYTES + 1;
    let error = load_hostile_leaf(
        leaf_page(
            &reference,
            u64::try_from(over_ceiling).expect("ceiling"),
            &vec![9; 32 * over_ceiling.div_ceil(STATE_PAGE_VALUE_CHUNK_BYTES)],
        ),
        &[],
    );
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    let error = load_hostile_leaf(leaf_page(&reference, u64::MAX, &[]), &[]);
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // Fewer chunk IDs than the declared length needs.
    let full = chunk_page(&vec![b'a'; STATE_PAGE_VALUE_CHUNK_BYTES]);
    let error = load_hostile_leaf(
        leaf_page(&reference, 3 * chunk, &hex_bytes(&full.page_id)),
        std::slice::from_ref(&full),
    );
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // More chunk IDs than the declared length needs.
    let ids = [&full.page_id, &full.page_id, &full.page_id]
        .into_iter()
        .flat_map(|page_id| hex_bytes(page_id))
        .collect::<Vec<_>>();
    let error = load_hostile_leaf(
        leaf_page(&reference, 2 * chunk, &ids),
        std::slice::from_ref(&full),
    );
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // Real chunk pages that are not the canonical split of the value. The
    // split check refuses them before the value is decoded or rebuilt.
    let short = chunk_page(&vec![b'a'; STATE_PAGE_VALUE_CHUNK_BYTES - 5]);
    let tail = chunk_page(&[b'a'; 15]);
    let ids = [&short.page_id, &tail.page_id]
        .into_iter()
        .flat_map(|page_id| hex_bytes(page_id))
        .collect::<Vec<_>>();
    let error = load_hostile_leaf(leaf_page(&reference, chunk + 10, &ids), &[short, tail]);
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(
        error.message.contains("canonical split"),
        "{}",
        error.message
    );

    // A chunk ID that names a page that is not a chunk.
    let not_a_chunk = StatePageBlob::new(b"{\"node\":\"leaf\"}".to_vec()).expect("page");
    let ids = [&full.page_id, &not_a_chunk.page_id]
        .into_iter()
        .flat_map(|page_id| hex_bytes(page_id))
        .collect::<Vec<_>>();
    let error = load_hostile_leaf(
        leaf_page(&reference, chunk + 1, &ids),
        &[full.clone(), not_a_chunk],
    );
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A chunk the provider does not hold.
    let mut ids = hex_bytes(&full.page_id);
    ids.extend([7; 32]);
    let error = load_hostile_leaf(
        leaf_page(&reference, chunk + 1, &ids),
        std::slice::from_ref(&full),
    );
    assert_eq!(error.code, ErrorCode::StatePageUnavailable);

    // Malformed pages: an unknown kind, an entry count the page cannot hold,
    // trailing bytes, and a page that is only the magic.
    let mut unknown = leaf_page(&reference, 2, b"{}");
    unknown[4] = 9;
    assert_eq!(
        load_hostile_leaf(unknown, &[]).code,
        ErrorCode::InvalidSnapshot
    );
    let mut counted = leaf_page(&reference, 2, b"{}");
    let count_at = 5 + domain_bytes().len() + 32;
    counted[count_at..count_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(
        load_hostile_leaf(counted, &[]).code,
        ErrorCode::InvalidSnapshot
    );
    let mut trailing = leaf_page(&reference, 2, b"{}");
    trailing.push(0);
    assert_eq!(
        load_hostile_leaf(trailing, &[]).code,
        ErrorCode::InvalidSnapshot
    );
    assert_eq!(
        load_hostile_leaf(b"\0cwp".to_vec(), &[]).code,
        ErrorCode::InvalidSnapshot
    );
}

/// One record crosses the inline-or-chunked threshold in both directions,
/// rewritten in phases 7, 10, and 12 of each boundary, while a phase-7
/// next-boundary writer runs on odd days. The live run, a fork, exact replay,
/// a sealed run, restores from its sealed segments (flat and paged), and
/// paged restores from a provider holding only the retention closure all
/// reach the same checkpoint hash.
mod threshold {
    use super::{PageStore, daily};
    use canwu_api::{
        BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
        BoundarySystemContract, Canwu, CanwuError, CompactedCanwu, DomainRecord, DomainRecordClass,
        DomainRecordDraft, DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation,
        DomainRecordRef, DomainRecordSchema, EvidenceCursor, PagedRestoreBudget,
        PagedSimulationCheckpoint, PluginRegistrar, STATE_PAGE_VALUE_CHUNK_BYTES, Scenario,
        SimDuration, SimTime, SimulationPlugin, SimulationView, StatePageRetentionLedger,
        StateVisibility, SystemCadence,
    };
    use serde_json::json;

    const PLUGIN: &str = "fixture-threshold";
    const MIB: usize = STATE_PAGE_VALUE_CHUNK_BYTES;
    /// The record's encoded length at the end of each day: exactly one chunk,
    /// so inline; one and two bytes over; a few hundred bytes; three chunks
    /// and a byte; exactly two chunks.
    const TARGETS: [usize; 6] = [MIB, MIB + 1, MIB + 2, 300, 3 * MIB + 1, 2 * MIB];

    type Handler =
        fn(&SimulationView<'_>, &BoundaryContext) -> Result<BoundaryProposal, CanwuError>;

    fn reference(name: &str) -> DomainRecordRef {
        DomainRecordRef {
            kind: DomainRecordKind::new(PLUGIN, name),
            id: name.to_owned(),
        }
    }

    fn encoded_len(version: u64) -> usize {
        serde_json::to_vec(&DomainRecord {
            reference: reference("big"),
            owner: PLUGIN.to_owned(),
            class: DomainRecordClass::Record,
            version,
            lifecycle: DomainRecordLifecycle::Active,
            payload: json!({ "text": "" }),
            references: Vec::new(),
        })
        .expect("encode record")
        .len()
    }

    fn day(context: &BoundaryContext) -> usize {
        usize::try_from(context.boundary_id.get().div_ceil(2)).expect("day")
    }

    fn write(
        view: &SimulationView<'_>,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<BoundaryProposal, CanwuError> {
        let record = DomainRecordDraft::new(reference(name), payload);
        let mutation = match view.domain_record(&reference(name))? {
            None => DomainRecordMutation::Create { record },
            Some(current) => DomainRecordMutation::Update {
                record,
                expected_version: current.version,
            },
        };
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::MutateRecord {
                mutation,
                summary: format!("Wrote {name}"),
            }],
            ..BoundaryProposal::default()
        })
    }

    /// Writes the text of the day plus two bytes; phases 10 and 12 each
    /// remove one, so the record ends the day at its target length.
    fn write_big(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let day = day(context);
        let version = view
            .domain_record(&reference("big"))?
            .map_or(0, |record| record.version);
        let letter = char::from(b'a' + u8::try_from(day).expect("day"));
        let mut text = letter
            .to_string()
            .repeat(TARGETS[day - 1] - encoded_len(version + 3));
        text.push_str("yy");
        write(view, "big", json!({ "text": text }))
    }

    fn trim_big(
        view: &SimulationView<'_>,
        _: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let current = view.domain_record(&reference("big"))?;
        let mut text = current.expect("written in phase 7").payload["text"]
            .as_str()
            .expect("text")
            .to_owned();
        text.pop();
        write(view, "big", json!({ "text": text }))
    }

    fn write_side(
        view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let day = day(context);
        if day.is_multiple_of(2) {
            return Ok(BoundaryProposal::default());
        }
        write(view, "side", json!({ "day": day }))
    }

    struct ThresholdPlugin;

    impl SimulationPlugin for ThresholdPlugin {
        fn name(&self) -> &'static str {
            PLUGIN
        }

        fn version(&self) -> &'static str {
            "1"
        }

        fn semantic_hash(&self) -> &'static str {
            "00000000000000000000000000000000000000000000000000000000000c0ffe"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            use BoundaryPhase::{
                DomainDeltaProposal, HistoricalCandidateEvaluation, StrategicAggregation,
            };
            use StateVisibility::{NextBoundary, SameBoundary};
            for name in ["big", "side"] {
                let kind = reference(name).kind;
                registrar.register_record_schema(DomainRecordSchema::new(
                    kind,
                    DomainRecordClass::Record,
                ))?;
            }
            let systems: [(&str, &str, BoundaryPhase, StateVisibility, Handler); 4] = [
                ("write", "big", DomainDeltaProposal, SameBoundary, write_big),
                (
                    "side",
                    "side",
                    DomainDeltaProposal,
                    NextBoundary,
                    write_side,
                ),
                (
                    "trim-10",
                    "big",
                    HistoricalCandidateEvaluation,
                    SameBoundary,
                    trim_big,
                ),
                (
                    "trim-12",
                    "big",
                    StrategicAggregation,
                    SameBoundary,
                    trim_big,
                ),
            ];
            for (name, record, phase, visibility, handler) in systems {
                let state =
                    DomainRecordSchema::new(reference(record).kind, DomainRecordClass::Record)
                        .state_key();
                let mut contract = BoundarySystemContract::new(name, phase, SystemCadence::Daily);
                contract.reads = vec![state.clone()];
                contract.writes = vec![state];
                contract.visibility = visibility;
                registrar.register_boundary_system(contract, handler)?;
            }
            Ok(())
        }
    }

    /// The Daily boundary of `day`, then one at the same time that admits its
    /// events, so that evidence can be sealed after any day.
    fn settle_day(mut settle: impl FnMut(BoundaryRequest), day: i64) {
        settle(daily(day));
        settle(BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day)));
    }

    fn new_run() -> Canwu {
        Canwu::new_with_plugins(
            91,
            Scenario::new(SimTime::EPOCH, Vec::new()),
            &[&ThresholdPlugin],
        )
        .expect("threshold run")
    }

    #[test]
    fn a_record_crossing_the_chunk_threshold_restores_and_replays_exactly() {
        let plugins: [&dyn SimulationPlugin; 1] = [&ThresholdPlugin];
        let mut live = new_run();
        let (mut segments, mut cursor) = (Vec::new(), EvidenceCursor::default());
        let store = PageStore::default();
        let mut previous: Option<PagedSimulationCheckpoint> = None;
        let (mut fork, mut hashes) = (None, Vec::new());
        for (day, target) in (1..).zip(TARGETS) {
            settle_day(
                |request| drop(live.settle_boundary(request).expect("settles")),
                day,
            );
            let big = live.domain_record(&reference("big")).expect("big");
            assert_eq!(serde_json::to_vec(big).expect("encode").len(), target);
            segments.push(live.journal_segment_since(cursor).expect("segment"));
            cursor = live.evidence_cursor().expect("cursor");
            let prepared = live
                .prepare_paged_checkpoint(previous.as_ref(), &store)
                .expect("paged checkpoint");
            prepared.store_and_verify(&store).expect("store pages");
            let mut ledger = StatePageRetentionLedger::default();
            let handle = ledger.prepare(&prepared.delta, &store).expect("retention");
            ledger.verify(&handle, &store).expect("verify retention");
            let closure = PageStore::default();
            for page_id in &ledger.handles[&handle].page_ids {
                let page = store.0.borrow()[page_id].clone();
                closure.0.borrow_mut().insert(page_id.clone(), page);
            }
            let restored = Canwu::from_paged_checkpoint_and_journal_with_plugins(
                &prepared.checkpoint,
                &closure,
                segments.clone(),
                &plugins,
                PagedRestoreBudget::DEFAULT,
            )
            .expect("restore from the retention closure alone");
            assert_eq!(
                restored.checkpoint_hash(),
                live.checkpoint_hash(),
                "day {day}"
            );
            previous = Some(prepared.checkpoint);
            hashes.push(live.checkpoint_hash().to_owned());
            if day == 5 {
                fork = Some(live.fork());
            }
        }

        let mut fork = fork.expect("forked on day 5");
        for day in 6..=6 {
            settle_day(
                |request| drop(fork.settle_boundary(request).expect("settles")),
                day,
            );
        }
        assert_eq!(fork.checkpoint_hash(), live.checkpoint_hash());
        let journal = serde_json::to_string(&live.replay_journal()).expect("journal");
        let replayed = Canwu::replay_from_journal_json(&plugins, &journal).expect("replay");
        assert_eq!(replayed.checkpoint_hash(), live.checkpoint_hash());

        // A run that seals on day 3 restores from its sealed segments, flat and
        // paged, and the paged restore continues identically to day 6.
        let mut sealed = new_run().into_compacted().expect("compact run");
        let (mut sealed_segments, sealed_store) = (Vec::new(), PageStore::default());
        let mut resumed: Option<CompactedCanwu> = None;
        for (day, hash) in (1..).zip(&hashes) {
            settle_day(
                |request| drop(sealed.settle_boundary(request).expect("settles")),
                day,
            );
            if let Some(resumed) = resumed.as_mut() {
                settle_day(
                    |request| drop(resumed.settle_boundary(request).expect("settles")),
                    day,
                );
                assert_eq!(resumed.checkpoint_hash(), hash, "day {day}");
            }
            assert_eq!(sealed.checkpoint_hash(), hash, "day {day}");
            if day != 3 {
                continue;
            }
            sealed_segments.push(sealed.seal_evidence().expect("seal").expect("sealable"));
            let flat = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
                sealed.checkpoint().expect("checkpoint"),
                sealed_segments.clone(),
                &plugins,
            )
            .expect("flat restore from sealed segments");
            assert_eq!(flat.checkpoint_hash(), hash);
            let prepared = sealed
                .prepare_paged_checkpoint(None, &sealed_store)
                .expect("sealed paged checkpoint");
            prepared
                .store_and_verify(&sealed_store)
                .expect("store pages");
            let paged = CompactedCanwu::from_paged_checkpoint_and_journal_with_plugins(
                &prepared.checkpoint,
                &sealed_store,
                sealed_segments.clone(),
                &plugins,
                PagedRestoreBudget::DEFAULT,
            )
            .expect("paged restore from sealed segments");
            assert_eq!(paged.checkpoint_hash(), hash);
            resumed = Some(paged);
        }
    }
}
