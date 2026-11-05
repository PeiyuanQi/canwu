//! The 0.22.0 page path kept as an oracle: generated ledgers and overlays
//! must page exactly as it paged, with the same read cuts, and the page and
//! read-cut costs must stay bounded by the page.

use crate::visits;
use crate::{
    HolderEntry, HolderLedgers, KnowledgeBodyDigest, KnowledgeCursor, KnowledgeHistoryView,
    KnowledgeOrigin, KnowledgeQuery, KnowledgeQueryError, KnowledgeQueryResult, KnowledgeReadCut,
    KnowledgeRecord, KnowledgeRecordBody, KnowledgeRecordView, KnowledgeSnapshot, KnowledgeSubject,
    KnowledgeSubjectTarget, MAX_KNOWLEDGE_PAGE_SIZE, StagedKnowledgeEntries, cursor_binding_hash,
    hash_material, holder_overlay_root, holder_projection_root, query_hash, to_view,
    validate_cursor,
};
use canwu_core::{
    BoundaryId, EventId, HolderKnowledgeRecordId, KnowledgeHolderRef, KnowledgeRecordId,
    KnowledgeRecordKind, KnowledgeSchemaId, PersonId,
};
use canwu_time::SimTime;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

// ---- The 0.22.0 path, verbatim but for names and visit counts. ----

#[derive(Serialize)]
struct HolderProjectionCutMaterial<'a> {
    holder: &'a KnowledgeHolderRef,
    full_history_views: Vec<KnowledgeRecordView>,
}

fn oracle_projection_root(
    holder: &KnowledgeHolderRef,
    ledger: &HolderLedgers,
) -> Result<String, KnowledgeQueryError> {
    let views = ledger.get(holder).map_or_else(Vec::new, |records| {
        records
            .entries()
            .map(|(entry, body)| {
                visits::view_hashed();
                to_view(holder, entry, body, records)
            })
            .collect()
    });
    hash_material(
        b"canwu.knowledge.holder-projection.v1",
        &HolderProjectionCutMaterial {
            holder,
            full_history_views: views,
        },
    )
}

fn oracle_query_records(
    ledger: &HolderLedgers,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
    read_cut: KnowledgeReadCut,
) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
    if query.limit == 0 || query.limit > MAX_KNOWLEDGE_PAGE_SIZE {
        return Err(KnowledgeQueryError::InvalidLimit);
    }
    let query_hash = query_hash(query)?;
    let binding_hash = cursor_binding_hash(&holder, &query_hash, &read_cut)?;
    if let Some(cursor) = &query.after {
        validate_cursor(cursor, &holder, &query_hash, &read_cut)?;
    }
    let Some(records) = ledger.get(&holder) else {
        if query.after.is_some() {
            return Err(KnowledgeQueryError::InvalidCursor);
        }
        return Ok(KnowledgeQueryResult {
            holder,
            read_cut,
            records: Vec::new(),
            next: None,
            retention: BTreeMap::new(),
        });
    };
    if records.entries().any(|(entry, body)| {
        visits::page_entry();
        entry.record != body.id
            || body.id.get() == 0
            || entry.local.get() == 0
            || entry
                .supersedes
                .iter()
                .chain(&entry.contradicts)
                .any(|related| !records.contains_key(related))
    }) {
        return Err(KnowledgeQueryError::InvalidLedger);
    }
    if let Some(cursor) = &query.after
        && records
            .id_of(cursor.record)
            .and_then(|id| records.body(&id))
            .is_none_or(|body| body.learned_at != cursor.learned_at)
    {
        return Err(KnowledgeQueryError::InvalidCursor);
    }
    let current_only = query.view == KnowledgeHistoryView::CurrentHeads;
    let mut candidates = records
        .entries()
        .filter(|(entry, body)| {
            visits::page_entry();
            (!current_only || records.is_current(&entry.record))
                && (query.schemas.is_empty() || query.schemas.contains(&body.schema))
                && query
                    .subjects
                    .iter()
                    .all(|subject| body.subjects.contains(subject))
                && query
                    .learned_after
                    .is_none_or(|time| body.learned_at > time)
                && query
                    .learned_at_or_before
                    .is_none_or(|time| body.learned_at <= time)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(entry, body)| (body.learned_at, entry.local));
    if let Some(cursor) = &query.after {
        candidates.retain(|(entry, body)| {
            (body.learned_at, entry.local) > (cursor.learned_at, cursor.record)
        });
    }
    let limit = usize::try_from(query.limit).unwrap_or(MAX_KNOWLEDGE_PAGE_SIZE as usize);
    let has_more = candidates.len() > limit;
    let page = candidates.into_iter().take(limit).collect::<Vec<_>>();
    let next = if has_more {
        page.last().map(|(entry, body)| KnowledgeCursor {
            holder: holder.clone(),
            query_hash: query_hash.clone(),
            read_cut: read_cut.clone(),
            binding_hash: binding_hash.clone(),
            learned_at: body.learned_at,
            record: entry.local,
        })
    } else {
        None
    };
    let views = page
        .iter()
        .map(|(entry, body)| to_view(&holder, entry, body, records))
        .collect();
    Ok(KnowledgeQueryResult {
        holder,
        read_cut,
        records: views,
        next,
        retention: BTreeMap::new(),
    })
}

fn oracle_query_current(
    records: &HolderLedgers,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
    boundary: Option<BoundaryId>,
) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
    let read_cut = KnowledgeReadCut {
        boundary,
        holder_projection_root: oracle_projection_root(&holder, records)?,
        holder_overlay_root: None,
    };
    oracle_query_records(records, holder, query, read_cut)
}

fn oracle_query_with_overlay(
    records: &HolderLedgers,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
    boundary: Option<BoundaryId>,
    overlay: &StagedKnowledgeEntries,
) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
    let merged = match overlay.get(&holder) {
        Some(staged) => records
            .with_staged(&holder, staged)
            .map_err(|_| KnowledgeQueryError::InvalidLedger)?,
        None => records.clone(),
    };
    let read_cut = KnowledgeReadCut {
        boundary,
        holder_projection_root: oracle_projection_root(&holder, records)?,
        holder_overlay_root: Some(holder_overlay_root(&holder, &merged, overlay)?),
    };
    oracle_query_records(&merged, holder, query, read_cut)
}

// ---- Generated ledgers. ----

/// A small deterministic generator, the same on every platform and run.
struct Sequence(u64);

impl Sequence {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.next(one_in) == 0
    }

    fn pick<T: Clone>(&mut self, values: &[T]) -> Option<T> {
        if values.is_empty() {
            return None;
        }
        let bound = u64::try_from(values.len()).unwrap();
        Some(values[usize::try_from(self.next(bound)).unwrap()].clone())
    }
}

const HOLDERS: u64 = 3;

fn schema(index: u64) -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(format!("fixture.pages{}", index % 2), "claim"),
        u32::try_from(index / 2 % 2 + 1).unwrap(),
    )
}

fn subject(index: u64) -> KnowledgeSubject {
    KnowledgeSubject {
        role: format!("line{index}"),
        target: KnowledgeSubjectTarget::Event(EventId::new(index + 1)),
    }
}

fn holder(index: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(index + 1))
}

fn minutes(value: u64) -> SimTime {
    SimTime::from_minutes(i64::try_from(value).unwrap())
}

fn body(
    id: u64,
    schema: KnowledgeSchemaId,
    subjects: Vec<KnowledgeSubject>,
    learned_at: SimTime,
) -> KnowledgeRecordBody {
    KnowledgeRecordBody {
        id: KnowledgeRecordId::new(id),
        schema,
        subjects,
        payload: serde_json::json!({ "id": id, "text": "x".repeat(text_bytes(id)) }),
        as_of: id.is_multiple_of(3).then(|| minutes(id % 7)),
        learned_at,
        confidence_per_mille: u16::try_from(id % 1_000).unwrap(),
        origin: KnowledgeOrigin {
            method: "fixture".to_owned(),
            evidence: Vec::new(),
        },
    }
}

/// A few bytes of payload text, and for one body in 37 about 4 KiB (3,796
/// to 4,396 bytes, exactly 4,096 among them), so a view's encoding crosses
/// the projection hash's 4 KiB stream buffer at varying offsets.
fn text_bytes(id: u64) -> usize {
    let bytes = if id.is_multiple_of(37) {
        3_796 + id / 37 % 7 * 100
    } else {
        id % 5
    };
    usize::try_from(bytes).unwrap()
}

fn held(ledgers: &HolderLedgers, holder: &KnowledgeHolderRef) -> Vec<KnowledgeRecordId> {
    ledgers
        .get(holder)
        .map(|ledger| ledger.keys().copied().collect())
        .unwrap_or_default()
}

/// Relations to some of the holder's held entries (or `extra` ones).
fn relations(
    sequence: &mut Sequence,
    targets: &[KnowledgeRecordId],
) -> (Vec<KnowledgeRecordId>, Vec<KnowledgeRecordId>) {
    let mut supersedes = Vec::new();
    let mut contradicts = Vec::new();
    for target in targets {
        match sequence.next(8) {
            0 => supersedes.push(*target),
            1 => contradicts.push(*target),
            _ => {}
        }
    }
    (supersedes, contradicts)
}

/// One runtime-shaped publication: a new body to one to three holders, or
/// an existing body given to a holder that does not hold it, which files an
/// entry below the holder's highest global ID.
fn publish(sequence: &mut Sequence, ledgers: &mut HolderLedgers, next_id: &mut u64) {
    if sequence.chance(3) && ledgers.body_count() > 0 {
        let bodies: Vec<_> = ledgers.bodies().map(|body| body.id).collect();
        let id = sequence.pick(&bodies).unwrap();
        let target = holder(sequence.next(HOLDERS));
        if ledgers
            .get(&target)
            .is_some_and(|ledger| ledger.contains_key(&id))
        {
            return;
        }
        let local = ledgers
            .get(&target)
            .map_or(1, |ledger| ledger.next_local().get());
        // An entry below the holder's highest global ID that names later
        // entries: removing one of those changes this entry's view.
        let targets = held(ledgers, &target);
        let (mut supersedes, mut contradicts) = relations(sequence, &targets);
        if let Some(named) = sequence.pick(&targets)
            && !supersedes.contains(&named)
            && !contradicts.contains(&named)
        {
            if sequence.chance(2) {
                supersedes.push(named);
            } else {
                contradicts.push(named);
            }
        }
        let entry = HolderEntry {
            record: id,
            local: HolderKnowledgeRecordId::new(local),
            supersedes,
            contradicts,
        };
        ledgers
            .insert_publication(
                std::iter::empty(),
                [(&target, std::slice::from_ref(&entry))],
            )
            .expect("an existing body is shared");
        return;
    }
    let id = *next_id;
    *next_id += 1;
    let subjects = (0..4).filter(|_| sequence.chance(2)).map(subject).collect();
    let published = body(
        id,
        schema(sequence.next(4)),
        subjects,
        minutes(sequence.next(12)),
    );
    let first = sequence.next(HOLDERS);
    let holders: std::collections::BTreeSet<u64> = (0..=sequence.next(3))
        .map(|offset| (first + offset) % HOLDERS)
        .collect();
    let mut entries = Vec::new();
    for index in holders {
        let target = holder(index);
        let local = ledgers
            .get(&target)
            .map_or(1, |ledger| ledger.next_local().get());
        let (supersedes, contradicts) = relations(sequence, &held(ledgers, &target));
        entries.push((
            target,
            HolderEntry {
                record: KnowledgeRecordId::new(id),
                local: HolderKnowledgeRecordId::new(local),
                supersedes,
                contradicts,
            },
        ));
    }
    let digest = KnowledgeBodyDigest::of(&published).unwrap();
    ledgers
        .insert_body(Arc::new(published), digest, entries)
        .expect("a runtime-shaped publication is admitted");
}

/// Entries a boundary stages for `target` beside the committed ledgers,
/// naming committed and earlier staged entries; now and then one names an
/// ID the holder does not hold, which both paths must refuse.
fn overlay(
    sequence: &mut Sequence,
    ledgers: &HolderLedgers,
    target: &KnowledgeHolderRef,
    first_id: u64,
) -> StagedKnowledgeEntries {
    let mut staged = BTreeMap::new();
    let first_local = ledgers
        .get(target)
        .map_or(1, |ledger| ledger.next_local().get());
    let mut nameable = held(ledgers, target);
    if sequence.chance(12) {
        nameable.push(KnowledgeRecordId::new(first_id + 50));
    }
    for offset in 0..sequence.next(4) {
        let id = first_id + offset;
        let (supersedes, contradicts) = relations(sequence, &nameable);
        let staged_body = body(
            id,
            schema(sequence.next(4)),
            (0..4).filter(|_| sequence.chance(2)).map(subject).collect(),
            minutes(sequence.next(14)),
        );
        let entry = HolderEntry {
            record: KnowledgeRecordId::new(id),
            local: HolderKnowledgeRecordId::new(first_local + offset),
            supersedes,
            contradicts,
        };
        staged.insert(KnowledgeRecordId::new(id), (Arc::new(staged_body), entry));
        nameable.push(KnowledgeRecordId::new(id));
    }
    let mut overlay = BTreeMap::new();
    overlay.insert(target.clone(), staged);
    overlay
}

fn random_query(sequence: &mut Sequence) -> KnowledgeQuery {
    let mut schemas: Vec<_> = (0..4).filter(|_| sequence.chance(3)).map(schema).collect();
    if sequence.chance(6)
        && let Some(first) = schemas.first().cloned()
    {
        schemas.push(first);
    }
    if sequence.chance(10) {
        schemas.push(KnowledgeSchemaId::new(
            KnowledgeRecordKind::new("fixture.unheld", "claim"),
            1,
        ));
    }
    let time = |sequence: &mut Sequence| {
        (!sequence.chance(2))
            .then(|| SimTime::from_minutes(i64::try_from(sequence.next(15)).unwrap() - 1))
    };
    KnowledgeQuery {
        schemas,
        subjects: (0..4).filter(|_| sequence.chance(5)).map(subject).collect(),
        learned_after: time(sequence),
        learned_at_or_before: if sequence.chance(2) {
            None
        } else {
            time(sequence)
        },
        view: if sequence.chance(2) {
            KnowledgeHistoryView::CurrentHeads
        } else {
            KnowledgeHistoryView::FullHistory
        },
        after: None,
        limit: match sequence.next(12) {
            0 => 0,
            1 => MAX_KNOWLEDGE_PAGE_SIZE + 1,
            2 => MAX_KNOWLEDGE_PAGE_SIZE,
            other => u32::try_from(other % 5 + 1).unwrap(),
        },
    }
}

/// Pages one query to its end on both paths, comparing every page, then
/// replays one of its cursors with tampering and against another query.
fn assert_pages_agree(
    sequence: &mut Sequence,
    ledgers: &HolderLedgers,
    target: &KnowledgeHolderRef,
    overlay: Option<&StagedKnowledgeEntries>,
) {
    let snapshot = KnowledgeSnapshot {
        actors: BTreeMap::new(),
        records: ledgers.clone(),
    };
    let boundary = Some(BoundaryId::new(7));
    let run = |query: &KnowledgeQuery| {
        let new = match overlay {
            Some(overlay) => snapshot.query_with_overlay(target.clone(), query, boundary, overlay),
            None => snapshot.query_current(target.clone(), query, boundary),
        };
        let old = match overlay {
            Some(overlay) => {
                oracle_query_with_overlay(ledgers, target.clone(), query, boundary, overlay)
            }
            None => oracle_query_current(ledgers, target.clone(), query, boundary),
        };
        assert_eq!(new, old, "{target:?} {query:?}");
        new
    };
    let mut query = random_query(sequence);
    let mut cursors = Vec::new();
    for _ in 0..400 {
        let Ok(page) = run(&query) else { break };
        let Some(next) = page.next else { break };
        cursors.push(next.clone());
        query.after = Some(next);
    }
    let Some(cursor) = sequence.pick(&cursors) else {
        return;
    };
    // The same cursor under another query, with a forged binding, and with
    // its position moved (the binding does not cover the position).
    let mut other = random_query(sequence);
    other.after = Some(cursor.clone());
    let _ = run(&other);
    let mut forged = cursor.clone();
    forged.binding_hash = "forged".to_owned();
    query.after = Some(forged);
    let _ = run(&query);
    let mut moved = cursor.clone();
    moved.record = HolderKnowledgeRecordId::new(sequence.next(40));
    moved.learned_at = minutes(sequence.next(13));
    query.after = Some(moved.clone());
    let _ = run(&query);
    // A caller-supplied cut never changes, so a cursor whose entry left
    // reaches the position check, which both paths make alike.
    let fixed = KnowledgeReadCut {
        boundary,
        holder_projection_root: "fixed".to_owned(),
        holder_overlay_root: None,
    };
    let mut fixed_query = query.clone();
    fixed_query.after = None;
    if let Ok(first) = snapshot.query(target.clone(), &fixed_query, fixed.clone())
        && let Some(mut next) = first.next
    {
        for position in [next.record, moved.record] {
            next.record = position;
            fixed_query.after = Some(next.clone());
            assert_eq!(
                snapshot.query(target.clone(), &fixed_query, fixed.clone()),
                oracle_query_records(ledgers, target.clone(), &fixed_query, fixed.clone()),
            );
        }
    }
}

fn assert_roots_agree(ledgers: &HolderLedgers) {
    for index in 0..=HOLDERS {
        let target = holder(index);
        assert_eq!(
            holder_projection_root(&target, ledgers),
            oracle_projection_root(&target, ledgers),
            "{target:?}"
        );
    }
}

/// On runtime-shaped ledgers with shared bodies, supersessions,
/// contradictions, retirements, and same-boundary overlays, every page,
/// `next` cursor, cursor refusal, and read cut equals 0.22.0's, whether the
/// running projection hash is fresh or stale, and after a decode. A cursor
/// from before a change is refused as 0.22.0 refused it.
#[test]
fn pages_and_read_cuts_equal_the_previous_path() {
    let mut sequence = Sequence(0x9a6e_5f01);
    let mut ledgers = HolderLedgers::new();
    let mut next_id = 1;
    let mut removed = 0;
    for step in 0..600 {
        let before = ledgers.clone();
        match sequence.next(12) {
            0..=6 => publish(&mut sequence, &mut ledgers, &mut next_id),
            7 => {
                let target = holder(sequence.next(HOLDERS));
                if let Some(id) = sequence.pick(&held(&ledgers, &target)) {
                    removed += usize::from(ledgers.remove_entry(&target, id).is_some());
                }
            }
            8 => {
                let target = holder(sequence.next(HOLDERS));
                if let Some(id) = sequence.pick(&held(&ledgers, &target)) {
                    removed += ledgers.retire_superseded_by(&target, id);
                }
            }
            9 if sequence.chance(4) => {
                let bodies: Vec<_> = ledgers.bodies().map(|body| body.id).collect();
                if let Some(id) = sequence.pick(&bodies) {
                    ledgers.remove_body(id);
                    removed += 1;
                }
            }
            _ => ledgers.refresh_commitment(),
        }
        assert_roots_agree(&ledgers);
        if step % 3 == 0 {
            let target = holder(sequence.next(HOLDERS + 1));
            assert_pages_agree(&mut sequence, &ledgers, &target, None);
            let staged = overlay(&mut sequence, &ledgers, &target, next_id + 1_000);
            assert_pages_agree(&mut sequence, &ledgers, &target, Some(&staged));
            // A cursor from the cut before this step is refused alike.
            let snapshot = KnowledgeSnapshot {
                actors: BTreeMap::new(),
                records: before.clone(),
            };
            let query = KnowledgeQuery {
                limit: 1,
                view: KnowledgeHistoryView::FullHistory,
                ..KnowledgeQuery::default()
            };
            if let Ok(page) = snapshot.query_current(target.clone(), &query, None)
                && let Some(cursor) = page.next
            {
                let stale = KnowledgeQuery {
                    after: Some(cursor),
                    ..query
                };
                assert_eq!(
                    KnowledgeSnapshot {
                        actors: BTreeMap::new(),
                        records: ledgers.clone(),
                    }
                    .query_current(target.clone(), &stale, None),
                    oracle_query_current(&ledgers, target, &stale, None),
                );
            }
        }
    }
    assert!(ledgers.entry_count() > 150, "the ledgers should grow");
    assert!(removed > 40, "the sequence should remove entries");
    let decoded: HolderLedgers =
        serde_json::from_value(serde_json::to_value(&ledgers).unwrap()).unwrap();
    assert_roots_agree(&decoded);
    for index in 0..HOLDERS {
        assert_pages_agree(&mut sequence, &decoded, &holder(index), None);
    }
}

/// The authoring path admits relations to IDs the holder does not hold, a
/// record that names itself, global ID zero, and replacing a body. Pages
/// refuse an inconsistent ledger exactly as 0.22.0 did, and the read cut
/// follows an entry whose relation comes to resolve.
#[test]
fn authoring_ledgers_are_refused_and_rooted_alike() {
    let mut sequence = Sequence(0x51de_0a11);
    let mut ledgers = HolderLedgers::new();
    let mut refused = 0;
    let mut highest = 0;
    for step in 0..500 {
        match sequence.next(10) {
            0..=5 => {
                // Mostly a new highest ID, which extends a fresh projection;
                // otherwise a replaced body or, rarely, ID zero. Relations
                // may name IDs still to come.
                let id = match sequence.next(40) {
                    0 => 0,
                    1..=14 => sequence.next(highest + 1),
                    _ => {
                        highest += 1;
                        highest
                    }
                };
                let target = holder(sequence.next(2));
                let named: Vec<_> = (0..sequence.next(3))
                    .map(|_| KnowledgeRecordId::new(sequence.next(highest + 5)))
                    .collect();
                let (supersedes, contradicts) = relations(&mut sequence, &named);
                let entry_body = body(
                    id,
                    schema(sequence.next(4)),
                    (0..3).filter(|_| sequence.chance(2)).map(subject).collect(),
                    minutes(sequence.next(10)),
                );
                ledgers.insert(KnowledgeRecord::assemble(
                    &target,
                    &HolderEntry {
                        record: entry_body.id,
                        local: HolderKnowledgeRecordId::new(1),
                        supersedes,
                        contradicts,
                    },
                    &entry_body,
                ));
            }
            6 => {
                let target = holder(sequence.next(2));
                if let Some(id) = sequence.pick(&held(&ledgers, &target)) {
                    ledgers.remove_entry(&target, id);
                }
            }
            _ => ledgers.refresh_commitment(),
        }
        assert_roots_agree(&ledgers);
        if step % 2 == 0 {
            let target = holder(sequence.next(2));
            let query = random_query(&mut sequence);
            let snapshot = KnowledgeSnapshot {
                actors: BTreeMap::new(),
                records: ledgers.clone(),
            };
            let new = snapshot.query_current(target.clone(), &query, None);
            refused += usize::from(new == Err(KnowledgeQueryError::InvalidLedger));
            assert_eq!(new, oracle_query_current(&ledgers, target, &query, None));
        }
    }
    assert!(
        refused > 10,
        "the sequence should reach inconsistent ledgers"
    );
}

// ---- Cost. ----

/// One holder with `count` entries of `schema`, entry `i` learned at
/// minute `i`, admitted as a runtime publishes, plus `others` entries of
/// another schema interleaved between them.
fn long_holder(count: u64, others: u64) -> (HolderLedgers, KnowledgeHolderRef) {
    let target = holder(0);
    let mut ledgers = HolderLedgers::new();
    let mut local = 1;
    let mut id = 1;
    for index in 1..=count {
        for (schema_index, minute) in
            std::iter::once((0, index)).chain((index <= others).then_some((1, index)))
        {
            let published = body(id, schema(schema_index), vec![subject(0)], minutes(minute));
            let digest = KnowledgeBodyDigest::of(&published).unwrap();
            ledgers
                .insert_body(
                    Arc::new(published),
                    digest,
                    [(
                        target.clone(),
                        HolderEntry {
                            record: KnowledgeRecordId::new(id),
                            local: HolderKnowledgeRecordId::new(local),
                            supersedes: Vec::new(),
                            contradicts: Vec::new(),
                        },
                    )],
                )
                .unwrap();
            id += 1;
            local += 1;
        }
    }
    ledgers.refresh_commitment();
    (ledgers, target)
}

/// A holder of 10,000 entries and a page of 50 whose `learned_after` is
/// past all but ten: deriving the read cut hashes no view and the page reads
/// at most the page plus one entry, in a `CurrentHeads` and a `FullHistory`
/// page, alone and beside 10,000 entries of another schema the query does
/// not name, from a cursor, and with a same-boundary overlay. 0.22.0 hashed
/// every view and read every entry twice. After a retirement the next cut
/// hashes the views from the checkpoint before it, and none once refreshed.
#[test]
fn a_page_costs_the_page_not_the_history() {
    for others in [0, 10_000] {
        assert_page_cost(others);
    }
}

fn assert_page_cost(others: u64) {
    let (ledgers, target) = long_holder(10_000, others);
    let held = 10_000 + others;
    let snapshot = KnowledgeSnapshot {
        actors: BTreeMap::new(),
        records: ledgers.clone(),
    };
    visits::reset();
    let root = holder_projection_root(&target, &ledgers).unwrap();
    assert_eq!(visits::counts(), (0, 0), "deriving the cut reads no entry");
    visits::reset();
    assert_eq!(root, oracle_projection_root(&target, &ledgers).unwrap());
    assert_eq!(visits::counts(), (0, held), "0.22.0 hashed every view");

    for view in [
        KnowledgeHistoryView::CurrentHeads,
        KnowledgeHistoryView::FullHistory,
    ] {
        let query = KnowledgeQuery {
            schemas: vec![schema(0)],
            learned_after: Some(minutes(9_990)),
            view,
            limit: 50,
            ..KnowledgeQuery::default()
        };
        visits::reset();
        let page = snapshot
            .query_current(target.clone(), &query, None)
            .unwrap();
        let (entries, hashed) = visits::counts();
        assert_eq!(page.records.len(), 10);
        assert_eq!(hashed, 0);
        assert!(entries <= 10 + 1, "{entries} entries read for a page of 10");
        visits::reset();
        assert_eq!(
            oracle_query_current(&ledgers, target.clone(), &query, None).unwrap(),
            page
        );
        assert_eq!(
            visits::counts(),
            (2 * held, held),
            "0.22.0 read the history"
        );

        // From a cursor: the second page of three.
        let first = snapshot
            .query_current(
                target.clone(),
                &KnowledgeQuery {
                    limit: 3,
                    ..query.clone()
                },
                None,
            )
            .unwrap();
        visits::reset();
        let second = snapshot
            .query_current(
                target.clone(),
                &KnowledgeQuery {
                    limit: 3,
                    after: first.next,
                    ..query.clone()
                },
                None,
            )
            .unwrap();
        assert_eq!(second.records.len(), 3);
        assert!(visits::counts().0 <= 3 + 1);

        assert_overlay_page_cost(&snapshot, &ledgers, &target, &query);
    }

    // A retirement 999 entries from the end: the stale cut re-hashes from
    // the checkpoint before it, the refresh keeps that, and the next cut
    // reads nothing.
    let mut retired = ledgers.clone();
    let late = held - 999;
    retired.remove_entry(&target, KnowledgeRecordId::new(late));
    visits::reset();
    let stale = holder_projection_root(&target, &retired).unwrap();
    let (_, hashed) = visits::counts();
    assert!(
        hashed < 999 + crate::projection::PROJECTION_CHECKPOINT_VIEWS,
        "{hashed} views hashed after a late retirement"
    );
    assert_eq!(stale, oracle_projection_root(&target, &retired).unwrap());
    retired.refresh_commitment();
    visits::reset();
    assert_eq!(holder_projection_root(&target, &retired).unwrap(), stale);
    assert_eq!(visits::counts(), (0, 0));
}

/// The page of [`assert_page_cost`] with three staged entries of the named
/// schema: the staged entries join the page, and the cut still hashes no
/// committed view.
fn assert_overlay_page_cost(
    snapshot: &KnowledgeSnapshot,
    ledgers: &HolderLedgers,
    target: &KnowledgeHolderRef,
    query: &KnowledgeQuery,
) {
    let staged: BTreeMap<_, _> = (0..3)
        .map(|offset| {
            let id = 30_000 + offset;
            let staged_body = body(id, schema(0), vec![subject(0)], minutes(20_000));
            let entry = HolderEntry {
                record: KnowledgeRecordId::new(id),
                local: HolderKnowledgeRecordId::new(20_001 + offset),
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            };
            (KnowledgeRecordId::new(id), (Arc::new(staged_body), entry))
        })
        .collect();
    let overlay = BTreeMap::from([(target.clone(), staged)]);
    visits::reset();
    let page = snapshot
        .query_with_overlay(target.clone(), query, None, &overlay)
        .unwrap();
    assert_eq!(page.records.len(), 13);
    let (entries, hashed) = visits::counts();
    assert_eq!(hashed, 0);
    assert!(entries <= 13 + 1);
    assert_eq!(
        oracle_query_with_overlay(ledgers, target.clone(), query, None, &overlay).unwrap(),
        page
    );
}
