//! Resource archive profile: what a cold lookup, an archive commit, and the
//! restore audit cost as the archive grows, and what one boundary costs
//! against a hot terminal backlog.
//!
//! Every archived record is a copy of one rejected operation outcome with its
//! own operation key, placed in the hot root directly so that building a large
//! archive does not pay one whole-root validation per record. Commits go
//! through `prepare_resource_archive` and `store_and_verify`; the archive
//! commit that a boundary applies is reproduced on the public state fields.
//! The backlog cases settle real boundaries through the resource plugin.

use canwu_api::{
    BoundaryRequest, Canwu, EntityRef, KnowledgeHolderRef, PersonId, Scenario, SimTime,
};
use canwu_resource::{
    PreparedResourceArchiveBatchV1, RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
    RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, ResourceAllocationRequestV1,
    ResourceArchiveIndexDirectoryV1, ResourceArchiveLookupPageV1, ResourceArchiveRetentionHandleV1,
    ResourceArchiveRetentionPhaseV1, ResourceArchiveStore, ResourceCancelDemandRequestV1,
    ResourceDemandId, ResourceError, ResourceLimitsV1, ResourceOperationKey,
    ResourceOperationOutcome, ResourceOperationOutcomeId, ResourceOperationRequestV1,
    ResourcePlugin, ResourceRevision, ResourceState, ResourceTerminalRecordKeyV1,
    archived_resource_operation_outcome, canonical_digest, enqueue_resource_allocation,
    resource_state, validate_resource_archive_store,
};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

/// An in-memory archive store that copies the bytes of every object it
/// serves and counts loads, load bytes, and first-time stored bytes.
#[derive(Default)]
struct ProfileStore {
    objects: RefCell<BTreeMap<(String, String), Vec<u8>>>,
    loads: Cell<u64>,
    loaded_bytes: Cell<u64>,
    stored_bytes: RefCell<BTreeMap<String, u64>>,
}

impl ResourceArchiveStore for ProfileStore {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        let key = (namespace.to_owned(), object_id.to_owned());
        if self
            .objects
            .borrow_mut()
            .insert(key, bytes.to_vec())
            .is_none()
        {
            *self
                .stored_bytes
                .borrow_mut()
                .entry(namespace.to_owned())
                .or_default() += bytes.len() as u64;
        }
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        self.loads.set(self.loads.get() + 1);
        let value = self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned();
        if let Some(value) = &value {
            self.loaded_bytes
                .set(self.loaded_bytes.get() + value.len() as u64);
        }
        Ok(value)
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Ok(())
    }
}

fn operation_key(label: &str, index: usize) -> Result<ResourceOperationKey, ResourceError> {
    ResourceOperationKey::new(format!("profile:{label}:{index}"))
}

/// One real operation: cancelling a demand that does not exist settles as a
/// durable rejection and leaves one terminal outcome.
fn reject(
    state: &mut ResourceState,
    label: &str,
    index: usize,
) -> Result<ResourceOperationOutcome, ResourceError> {
    state.apply_operation(&ResourceOperationRequestV1::CancelDemand(
        ResourceCancelDemandRequestV1 {
            operation_key: operation_key(label, index)?,
            demand: ResourceDemandId::new(format!("profile:missing:{index}"))?,
            expected_demand_revision: ResourceRevision::INITIAL,
        },
    ))
}

fn template() -> Result<ResourceOperationOutcome, Box<dyn Error>> {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical())?;
    Ok(reject(&mut state, "template", 0)?)
}

/// Adds one hot terminal outcome: `template` under its own operation key.
fn add_hot_outcome(
    state: &mut ResourceState,
    template: &ResourceOperationOutcome,
    index: usize,
) -> Result<(), ResourceError> {
    let mut outcome = template.clone();
    outcome.operation_key = operation_key("archived", index)?;
    outcome.id = ResourceOperationOutcomeId::new(format!(
        "resource:outcome:{}:{}",
        outcome.operation_key.as_str().replace(':', "-"),
        &outcome.request_digest[..16]
    ))?;
    outcome.sequence = state.next_admitted_sequence;
    state.next_admitted_sequence += 1;
    outcome.semantic_digest.clear();
    outcome.semantic_digest = canonical_digest("canwu.resource.operation-outcome.v1", &outcome)?;
    state.terminal_archive_candidates.insert(
        outcome.sequence,
        ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone()),
    );
    state
        .outcomes
        .insert(outcome.operation_key.clone(), outcome);
    Ok(())
}

/// Moves the archive head to a stored commit, as the commit boundary does.
fn apply_commit(
    state: &mut ResourceState,
    prepared: &PreparedResourceArchiveBatchV1,
) -> Result<(), ResourceError> {
    state.outcomes.clear();
    state.terminal_archive_candidates.clear();
    state.archive_head.revision += 1;
    state.archive_head.directory_root = Some(prepared.directory.id.clone());
    state.archive_head.archived_record_count += prepared.selected.len() as u64;
    state.archive_head.semantic_digest.clear();
    state.archive_head.semantic_digest =
        canonical_digest("canwu.resource.archive-head.v1", &state.archive_head)?;
    Ok(())
}

fn milliseconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

/// Pages and bytes of the lookup index the archive head reaches.
fn live_lookup_index(
    state: &ResourceState,
    store: &ProfileStore,
) -> Result<(u64, u64), Box<dyn Error>> {
    let root = state
        .archive_head
        .directory_root
        .clone()
        .ok_or("the profile archive has no head")?;
    let objects = store.objects.borrow();
    let directory: ResourceArchiveIndexDirectoryV1 = serde_json::from_slice(
        objects
            .get(&(RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(), root))
            .ok_or("the head directory is missing")?,
    )?;
    let mut pending = vec![directory.lookup_root];
    let (mut pages, mut bytes) = (0_u64, 0_u64);
    while let Some(id) = pending.pop() {
        let raw = objects
            .get(&(RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE.to_owned(), id))
            .ok_or("a lookup page is missing")?;
        pages += 1;
        bytes += raw.len() as u64;
        let page: ResourceArchiveLookupPageV1 = serde_json::from_slice(raw)?;
        pending.extend(page.children.into_values());
    }
    Ok((pages, bytes))
}

/// Archives `commits` commits of `per_commit` records and measures the
/// archive they leave.
fn archive_case(
    template: &ResourceOperationOutcome,
    commits: usize,
    per_commit: usize,
    audit: bool,
) -> Result<Value, Box<dyn Error>> {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = per_commit.max(limits.max_archive_candidates);
    limits.max_archive_maintenance_receipts = limits
        .max_archive_candidates
        .max(limits.max_archive_maintenance_receipts);
    let mut state = ResourceState::empty(limits)?;
    let store = ProfileStore::default();
    let (mut last_prepare_ms, mut last_store_and_verify_ms) = (0.0, 0.0);
    let (mut lookup_pages_written, mut lookup_pages_superseded) = (0_usize, 0_usize);
    for commit in 0..commits {
        for record in 0..per_commit {
            add_hot_outcome(&mut state, template, commit * per_commit + record)?;
        }
        let started = Instant::now();
        let prepared = state.prepare_resource_archive(per_commit, &store)?;
        last_prepare_ms = milliseconds(started);
        let started = Instant::now();
        prepared.store_and_verify(&store)?;
        last_store_and_verify_ms = milliseconds(started);
        lookup_pages_written += prepared.lookup_pages.len();
        lookup_pages_superseded += prepared.superseded_lookup_pages.len();
        apply_commit(&mut state, &prepared)?;
    }
    let records = commits * per_commit;

    // A miss: keys the archive never held, each on its own index path.
    let absent = (0..2_000)
        .map(|index| operation_key("absent", index))
        .collect::<Result<Vec<_>, _>>()?;
    store.loads.set(0);
    store.loaded_bytes.set(0);
    let started = Instant::now();
    let mut lookups = 0_usize;
    while lookups < absent.len() || started.elapsed().as_millis() < 300 {
        let key = &absent[lookups % absent.len()];
        if archived_resource_operation_outcome(&state, &store, key)?.is_some() {
            return Err("a key that was never archived resolved".into());
        }
        lookups += 1;
    }
    let miss_microseconds = started.elapsed().as_secs_f64() * 1_000_000.0 / lookups as f64;
    let miss_objects = store.loads.get() as f64 / lookups as f64;
    let miss_bytes = store.loaded_bytes.get() as f64 / lookups as f64;

    // A hit on the oldest record.
    let oldest = operation_key("archived", 0)?;
    let started = Instant::now();
    let hit = archived_resource_operation_outcome(&state, &store, &oldest)?;
    let oldest_hit_microseconds = started.elapsed().as_secs_f64() * 1_000_000.0;
    if hit.is_none() {
        return Err("the oldest archived key did not resolve".into());
    }

    let (live_pages, live_bytes) = live_lookup_index(&state, &store)?;
    let stored = store.stored_bytes.borrow();
    let lookup_bytes = stored
        .get(RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE)
        .copied()
        .unwrap_or_default();
    let other_bytes: u64 = stored
        .iter()
        .filter(|(namespace, _)| namespace.as_str() != RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE)
        .map(|(_, bytes)| *bytes)
        .sum();
    let audit_ms = if audit {
        let started = Instant::now();
        validate_resource_archive_store(&state, &store)?;
        Some(milliseconds(started))
    } else {
        None
    };
    Ok(json!({
        "archived_records": records,
        "commits": commits,
        "records_per_commit": per_commit,
        "miss_microseconds": miss_microseconds,
        "miss_objects_read": miss_objects,
        "miss_bytes_read": miss_bytes,
        "oldest_hit_microseconds": oldest_hit_microseconds,
        "last_prepare_ms": last_prepare_ms,
        "last_store_and_verify_ms": last_store_and_verify_ms,
        "lookup_bytes_written_per_record": lookup_bytes as f64 / records as f64,
        "other_bytes_written_per_record": other_bytes as f64 / records as f64,
        "lookup_pages_written": lookup_pages_written,
        "lookup_pages_superseded": lookup_pages_superseded,
        "live_lookup_pages": live_pages,
        "live_lookup_bytes_per_record": live_bytes as f64 / records as f64,
        "restore_audit_ms": audit_ms,
    }))
}

/// One boundary against a root that holds `backlog` hot terminal records.
///
/// Each sampled boundary settles one requester-scoped allocation pass through
/// the resource plugin. The requester owns no demand, so the pass is a
/// durable rejection: one operation, one new terminal outcome. Only
/// `settle_boundary` is timed, and the evidence that boundary appends to the
/// journal is measured as encoded JSON. The same operation applied to a
/// detached copy of the root is reported as the state-level share.
fn backlog_case(
    template: &ResourceOperationOutcome,
    backlog: usize,
) -> Result<Value, Box<dyn Error>> {
    const SAMPLES: usize = 5;
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES;
    limits.max_archive_maintenance_receipts = ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES;
    let mut state = ResourceState::empty(limits)?;
    for index in 0..backlog {
        add_hot_outcome(&mut state, template, index)?;
    }
    state.validate()?;
    let resource_root_bytes = serde_json::to_vec(&state)?.len();

    let mut detached = state.clone();
    let started = Instant::now();
    for index in 0..SAMPLES {
        reject(&mut detached, "detached", index)?;
    }
    let state_operation_ms = milliseconds(started) / SAMPLES as f64;

    let requester = KnowledgeHolderRef::Person(PersonId::new(1));
    let plugin = ResourcePlugin::default();
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![state.into_record()?]);
    let mut canwu = Canwu::new_with_plugins(0x00CA_4E55, scenario, &[&plugin])?;
    let mut boundary_ms = Vec::with_capacity(SAMPLES);
    let mut journal_bytes = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let at = canwu.time();
        let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
        enqueue_resource_allocation(
            &mut canwu,
            at,
            &requester,
            &ResourceAllocationRequestV1 {
                operation_key: operation_key("boundary", index)?,
                expected_state_revision: root.state_revision,
                at,
                candidate_limit: 1,
            },
        )?;
        let cursor = canwu.evidence_cursor()?;
        let started = Instant::now();
        canwu.settle_boundary(BoundaryRequest::at(at))?;
        boundary_ms.push(milliseconds(started));
        journal_bytes.push(serde_json::to_vec(&canwu.journal_segment_since(cursor)?)?.len());
    }
    let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
    if root.outcomes.len() != backlog + SAMPLES {
        return Err("a sampled boundary did not settle its operation".into());
    }
    boundary_ms.sort_by(f64::total_cmp);
    journal_bytes.sort_unstable();
    Ok(json!({
        "hot_terminal_records": backlog,
        "resource_root_bytes": resource_root_bytes,
        "boundary_ms": boundary_ms[SAMPLES / 2],
        "journal_bytes_per_boundary": journal_bytes[SAMPLES / 2],
        "state_operation_ms": state_operation_ms,
    }))
}

struct Options {
    preset: String,
    machine: String,
    recorded_on: String,
    output: Option<PathBuf>,
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        preset: "smoke".to_owned(),
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
        output: None,
    };
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| arguments.next().ok_or(format!("{name} requires a value"));
        match argument.as_str() {
            "--preset" => options.preset = value("--preset")?,
            "--machine" => options.machine = value("--machine")?,
            "--recorded-on" => options.recorded_on = value("--recorded-on")?,
            "--output" => options.output = Some(PathBuf::from(value("--output")?)),
            "--help" | "-h" => {
                println!(
                    "Canwu resource archive profile\n\n\
                     --preset smoke|full\n\
                     --machine LABEL --recorded-on DATE --output PATH"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }
    Ok(options)
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// BLAKE3 over the measured sources: the resource extension and this file.
fn source_content_hash() -> String {
    let root = PathBuf::from(command_output("git", &["rev-parse", "--show-toplevel"]));
    let mut files =
        vec![root.join("benchmarks/performance-harness/src/bin/resource-archive-profile.rs")];
    if let Ok(entries) = fs::read_dir(root.join("crates/extensions/canwu-resource/src")) {
        files.extend(entries.flatten().map(|entry| entry.path()));
    }
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for path in files {
        if let Ok(relative) = path.strip_prefix(&root)
            && let Ok(bytes) = fs::read(&path)
        {
            hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
            hasher.update(&[0]);
            hasher.update(&bytes);
            hasher.update(&[0]);
        }
    }
    hasher.finalize().to_hex().to_string()
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = options()?;
    // (commits, records per commit, run the restore audit)
    let (archives, backlogs): (&[(usize, usize, bool)], &[usize]) = match options.preset.as_str() {
        "smoke" => (
            &[
                (37, 27, true),
                (4, 256, true),
                (100, 1, true),
                (2_049, 1, true),
            ],
            &[0, 256, 1_024],
        ),
        "full" => (
            &[
                (37, 27, true),
                (370, 27, true),
                (3_704, 27, true),
                (4, 256, true),
                (39, 256, true),
                (391, 256, true),
                (3_907, 256, true),
                (100, 1, true),
                (1_000, 1, true),
                (2_048, 1, true),
                (2_049, 1, true),
                (10_000, 1, true),
                (120_000, 1, true),
                (25, 4_096, true),
            ],
            &[0, 256, 1_024, 2_048, 4_000],
        ),
        other => return Err(format!("unsupported preset: {other}").into()),
    };
    let template = template()?;
    let mut archive_cases = Vec::with_capacity(archives.len());
    for (commits, per_commit, audit) in archives {
        eprintln!("archive case: {commits} commits of {per_commit}");
        archive_cases.push(archive_case(&template, *commits, *per_commit, *audit)?);
    }
    let mut backlog_cases = Vec::with_capacity(backlogs.len());
    for backlog in backlogs {
        eprintln!("backlog case: {backlog} hot terminal records");
        backlog_cases.push(backlog_case(&template, *backlog)?);
    }
    let report = json!({
        "benchmark": "resource-archive-profile",
        "preset": options.preset,
        "machine": options.machine,
        "recorded_on": options.recorded_on,
        "source": {
            "git_commit": command_output("git", &["rev-parse", "HEAD"]),
            "git_dirty": !command_output("git", &["status", "--porcelain"]).is_empty(),
            "content_hash": source_content_hash(),
            "rustc": command_output("rustc", &["-vV"]),
        },
        "archive_cases": archive_cases,
        "backlog_cases": backlog_cases,
    });
    let text = serde_json::to_string_pretty(&report)?;
    match options.output {
        Some(path) => fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}
