//! Content-addressed state pages used by the Format 8 checkpoint boundary.
//!
//! Pages are deliberately domain-neutral: the runtime commits the canonical
//! bytes and a host decides where (and whether) to physically store them. A
//! page ID is the hash of the canonical uncompressed bytes, so compression or
//! blob placement cannot change semantic identity.

use super::{ArchiveStoreOutcome, CanwuError, ErrorCode, canonical_byte_hash};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const STATE_PAGE_FORMAT_VERSION: u32 = 1;
pub const STATE_PAGE_CODEC: &str = "raw-canonical-v1";
pub const MAX_STATE_PAGE_BYTES: usize = 4 * 1024 * 1024;
/// Hard predecode cap for one initial or incremental Format-8 page graph.
/// A one-million-entry non-collision Patricia map can contain `2N - 1`
/// logical node pages; this leaves bounded room for its manifest and compact
/// decision buckets without making the count unbounded.
pub const MAX_STATE_DELTA_PAGES: usize = 4_194_304;
pub const STATE_PAGE_RETENTION_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatePageBlob {
    pub format_version: u32,
    pub page_id: String,
    pub codec: String,
    pub decoded_bytes: u64,
    pub bytes: Vec<u8>,
}

impl StatePageBlob {
    pub fn new(bytes: Vec<u8>) -> Result<Self, CanwuError> {
        if bytes.is_empty() || bytes.len() > MAX_STATE_PAGE_BYTES {
            return Err(page_error("state page bytes exceed the bounded page limit"));
        }
        let decoded_bytes = u64::try_from(bytes.len())
            .map_err(|_| page_error("state page byte count is not representable"))?;
        let page_id = state_page_id(&bytes);
        Ok(Self {
            format_version: STATE_PAGE_FORMAT_VERSION,
            page_id,
            codec: STATE_PAGE_CODEC.to_owned(),
            decoded_bytes,
            bytes,
        })
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != STATE_PAGE_FORMAT_VERSION {
            return Err(page_error(format!(
                "state page format {} is unsupported; expected {STATE_PAGE_FORMAT_VERSION}",
                self.format_version
            )));
        }
        if self.codec != STATE_PAGE_CODEC {
            return Err(page_error(
                "state page codec is not the canonical runtime codec",
            ));
        }
        if self.bytes.is_empty() || self.bytes.len() > MAX_STATE_PAGE_BYTES {
            return Err(page_error("state page bytes exceed the bounded page limit"));
        }
        if self.decoded_bytes != self.bytes.len() as u64 {
            return Err(page_error("state page decoded byte count is inconsistent"));
        }
        if self.page_id != state_page_id(&self.bytes) {
            return Err(page_error(
                "state page ID does not match its canonical bytes",
            ));
        }
        Ok(())
    }
}

#[must_use]
pub fn state_page_id(bytes: &[u8]) -> String {
    canonical_byte_hash("canwu.state-page.v1", bytes)
}

pub trait StatePageProvider {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError>;
}

pub trait StatePageStore: StatePageProvider {
    fn store_state_page(&self, page: &StatePageBlob) -> Result<ArchiveStoreOutcome, CanwuError>;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatePageRetentionPhase {
    Prepared,
    Verified,
    DurableIngress,
    Committed,
    Abandoned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatePageRetentionHandle {
    pub format_version: u32,
    pub handle_id: String,
    pub source_root: String,
    pub target_root: String,
    pub page_ids: BTreeSet<String>,
    pub prepared_epoch: u64,
    pub phase: StatePageRetentionPhase,
}

/// Persistable host-side mark/sweep interlock for content-addressed state
/// pages. Preparing, verifying, or durably enqueueing a root protects every
/// declared reachable page across process restart. A committed root takes over
/// that lease atomically before the transient handle may disappear.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatePageRetentionLedger {
    pub format_version: u32,
    pub gc_epoch: u64,
    pub handles: BTreeMap<String, StatePageRetentionHandle>,
    pub committed_roots: BTreeMap<String, BTreeSet<String>>,
}

impl Default for StatePageRetentionLedger {
    fn default() -> Self {
        Self {
            format_version: STATE_PAGE_RETENTION_FORMAT_VERSION,
            gc_epoch: 0,
            handles: BTreeMap::new(),
            committed_roots: BTreeMap::new(),
        }
    }
}

impl StatePageRetentionLedger {
    pub fn prepare(
        &mut self,
        delta: &PreparedStateDelta,
        provider: &dyn StatePageProvider,
    ) -> Result<String, CanwuError> {
        self.validate()?;
        delta.validate()?;
        let reachable_page_ids = state_page_closure(delta, provider)?;
        for page_id in &reachable_page_ids {
            validate_hash(page_id, "retained state page ID")?;
        }
        let handle_id = canonical_byte_hash(
            "canwu.state-page-retention-handle.v1",
            &serde_json::to_vec(&(
                &delta.token_hash,
                &delta.source_root,
                &delta.target_root,
                &reachable_page_ids,
                self.gc_epoch,
            ))
            .map_err(|error| page_error(format!("cannot encode retention handle: {error}")))?,
        );
        let handle = StatePageRetentionHandle {
            format_version: STATE_PAGE_RETENTION_FORMAT_VERSION,
            handle_id: handle_id.clone(),
            source_root: delta.source_root.clone(),
            target_root: delta.target_root.clone(),
            page_ids: reachable_page_ids,
            prepared_epoch: self.gc_epoch,
            phase: StatePageRetentionPhase::Prepared,
        };
        if let Some(existing) = self.handles.get(&handle_id) {
            if existing != &handle {
                return Err(page_error(
                    "state page retention handle collides with different content",
                ));
            }
            return Ok(handle_id);
        }
        self.handles.insert(handle_id.clone(), handle);
        Ok(handle_id)
    }

    pub fn verify(
        &mut self,
        handle_id: &str,
        provider: &dyn StatePageProvider,
    ) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get(handle_id)
            .cloned()
            .ok_or_else(|| page_error("state page retention handle is unknown"))?;
        if !matches!(
            handle.phase,
            StatePageRetentionPhase::Prepared | StatePageRetentionPhase::Verified
        ) {
            return Err(page_error(
                "state page retention handle cannot enter verified state",
            ));
        }
        for page_id in &handle.page_ids {
            let page = provider.load_state_page(page_id)?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StatePageUnavailable,
                    "retained state page is unavailable",
                )
            })?;
            page.validate()?;
            if page.page_id != *page_id {
                return Err(page_error("retained state page identity changed"));
            }
        }
        let observed =
            state_page_closure_for_root(&handle.target_root, provider, &BTreeMap::new())?;
        if observed != handle.page_ids {
            return Err(page_error(
                "retained state-page closure changed after preparation",
            ));
        }
        self.handles
            .get_mut(handle_id)
            .ok_or_else(|| page_error("state page retention handle disappeared during verify"))?
            .phase = StatePageRetentionPhase::Verified;
        Ok(())
    }

    pub fn mark_durable_ingress(&mut self, handle_id: &str) -> Result<(), CanwuError> {
        self.transition(
            handle_id,
            StatePageRetentionPhase::Verified,
            StatePageRetentionPhase::DurableIngress,
        )
    }

    pub fn commit(&mut self, handle_id: &str) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get(handle_id)
            .cloned()
            .ok_or_else(|| page_error("state page retention handle is unknown"))?;
        if handle.phase != StatePageRetentionPhase::DurableIngress
            && handle.phase != StatePageRetentionPhase::Committed
        {
            return Err(page_error(
                "only durable maintenance ingress may commit a state-page root",
            ));
        }
        if let Some(existing) = self.committed_roots.get(&handle.target_root)
            && existing != &handle.page_ids
        {
            return Err(page_error(
                "committed state root is bound to different reachable pages",
            ));
        }
        self.committed_roots
            .insert(handle.target_root.clone(), handle.page_ids.clone());
        self.handles
            .get_mut(handle_id)
            .ok_or_else(|| page_error("state page retention handle disappeared during commit"))?
            .phase = StatePageRetentionPhase::Committed;
        Ok(())
    }

    pub fn abandon(&mut self, handle_id: &str) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get_mut(handle_id)
            .ok_or_else(|| page_error("state page retention handle is unknown"))?;
        if matches!(
            handle.phase,
            StatePageRetentionPhase::DurableIngress | StatePageRetentionPhase::Committed
        ) {
            return Err(page_error(
                "durable or committed state-page retention cannot be abandoned",
            ));
        }
        handle.phase = StatePageRetentionPhase::Abandoned;
        Ok(())
    }

    pub fn release_committed_root(&mut self, root: &str) -> Result<(), CanwuError> {
        validate_hash(root, "released state root")?;
        self.committed_roots.remove(root);
        self.handles.retain(|_, handle| {
            !(handle.phase == StatePageRetentionPhase::Committed && handle.target_root == root)
        });
        self.validate()?;
        Ok(())
    }

    pub fn begin_gc_epoch(&mut self) -> Result<u64, CanwuError> {
        self.gc_epoch = self
            .gc_epoch
            .checked_add(1)
            .ok_or_else(|| page_error("state-page GC epoch is exhausted"))?;
        Ok(self.gc_epoch)
    }

    #[must_use]
    pub fn reachable_page_ids(&self) -> BTreeSet<String> {
        let mut reachable = self
            .committed_roots
            .values()
            .flat_map(|pages| pages.iter().cloned())
            .collect::<BTreeSet<_>>();
        for handle in self.handles.values().filter(|handle| {
            !matches!(
                handle.phase,
                StatePageRetentionPhase::Abandoned | StatePageRetentionPhase::Committed
            )
        }) {
            reachable.extend(handle.page_ids.iter().cloned());
        }
        reachable
    }

    #[must_use]
    pub fn sweep_candidates(
        &self,
        all_page_ids: impl IntoIterator<Item = String>,
    ) -> BTreeSet<String> {
        let reachable = self.reachable_page_ids();
        all_page_ids
            .into_iter()
            .filter(|page_id| !reachable.contains(page_id))
            .collect()
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != STATE_PAGE_RETENTION_FORMAT_VERSION {
            return Err(page_error("unsupported state-page retention format"));
        }
        for (handle_id, handle) in &self.handles {
            validate_hash(handle_id, "state-page retention handle ID")?;
            validate_hash(&handle.source_root, "retention source root")?;
            validate_hash(&handle.target_root, "retention target root")?;
            if handle.format_version != STATE_PAGE_RETENTION_FORMAT_VERSION
                || handle.handle_id != *handle_id
                || handle.page_ids.is_empty()
                || handle.prepared_epoch > self.gc_epoch
            {
                return Err(page_error("state-page retention handle is inconsistent"));
            }
            for page_id in &handle.page_ids {
                validate_hash(page_id, "retained state page ID")?;
            }
            if handle.phase == StatePageRetentionPhase::Committed
                && self.committed_roots.get(&handle.target_root) != Some(&handle.page_ids)
            {
                return Err(page_error(
                    "committed retention handle did not transfer its lease",
                ));
            }
        }
        for (root, pages) in &self.committed_roots {
            validate_hash(root, "committed state root")?;
            if pages.is_empty() {
                return Err(page_error("committed state root has no reachable pages"));
            }
            for page_id in pages {
                validate_hash(page_id, "committed state page ID")?;
            }
        }
        Ok(())
    }

    fn transition(
        &mut self,
        handle_id: &str,
        expected: StatePageRetentionPhase,
        next: StatePageRetentionPhase,
    ) -> Result<(), CanwuError> {
        let handle = self
            .handles
            .get_mut(handle_id)
            .ok_or_else(|| page_error("state page retention handle is unknown"))?;
        if handle.phase == next {
            return Ok(());
        }
        if handle.phase != expected {
            return Err(page_error("state page retention transition is invalid"));
        }
        handle.phase = next;
        Ok(())
    }
}

fn state_page_closure(
    delta: &PreparedStateDelta,
    provider: &dyn StatePageProvider,
) -> Result<BTreeSet<String>, CanwuError> {
    let new_pages = delta
        .new_pages
        .iter()
        .map(|page| (page.page_id.clone(), page.clone()))
        .collect::<BTreeMap<_, _>>();
    let reachable = state_page_closure_for_root(&delta.target_root, provider, &new_pages)?;
    if delta
        .new_pages
        .iter()
        .any(|page| !reachable.contains(&page.page_id))
    {
        return Err(page_error(
            "prepared state delta contains a page outside the target-root closure",
        ));
    }
    Ok(reachable)
}

fn state_page_closure_for_root(
    root: &str,
    provider: &dyn StatePageProvider,
    new_pages: &BTreeMap<String, StatePageBlob>,
) -> Result<BTreeSet<String>, CanwuError> {
    let mut reachable = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(page_id) = pending.pop() {
        if !reachable.insert(page_id.clone()) {
            continue;
        }
        if reachable.len() > MAX_STATE_DELTA_PAGES {
            return Err(page_error("state-page closure exceeds the hard page limit"));
        }
        let page = match new_pages.get(&page_id).cloned() {
            Some(page) => page,
            None => provider.load_state_page(&page_id)?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StatePageUnavailable,
                    "state-page closure references an unavailable page",
                )
            })?,
        };
        page.validate()?;
        if page.page_id != page_id {
            return Err(page_error(
                "state-page closure provider changed page identity",
            ));
        }
        let value = serde_json::from_slice::<serde_json::Value>(&page.bytes).map_err(|error| {
            page_error(format!(
                "state page cannot be decoded for reachability: {error}"
            ))
        })?;
        pending.extend(state_page_children(&value)?);
    }
    Ok(reachable)
}

fn state_page_children(value: &serde_json::Value) -> Result<Vec<String>, CanwuError> {
    let Some(object) = value.as_object() else {
        return Ok(Vec::new());
    };
    let mut children = Vec::new();
    if object.contains_key("checkpoint_without_paged_state")
        && object.contains_key("domain_records")
    {
        let records = object
            .get("domain_records")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| page_error("paged checkpoint domain roots are malformed"))?;
        for field in [
            "primary",
            "reverse_references",
            "successor_of",
            "predecessors_of",
        ] {
            if let Some(page_id) = records.get(field).and_then(serde_json::Value::as_str) {
                children.push(page_id.to_owned());
            }
        }
        if let Some(page_id) = object
            .get("decision_manifest_page_id")
            .and_then(serde_json::Value::as_str)
        {
            children.push(page_id.to_owned());
        }
    } else if let Some(page_id) = object
        .get("hot_page_id")
        .and_then(serde_json::Value::as_str)
    {
        children.push(page_id.to_owned());
        if let Some(pages) = object
            .get("archive_directory_page_ids")
            .and_then(serde_json::Value::as_array)
        {
            children.extend(
                pages
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned),
            );
        }
    } else if let Some(pages) = object
        .get("archive_bucket_pages")
        .and_then(serde_json::Value::as_array)
    {
        children.extend(pages.iter().filter_map(|entry| {
            entry
                .as_array()
                .and_then(|pair| pair.get(1))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        }));
    } else if object.get("node").and_then(serde_json::Value::as_str) == Some("branch") {
        for field in ["left_page", "right_page"] {
            let page_id = object
                .get(field)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| page_error("Patricia branch page is missing a child"))?;
            children.push(page_id.to_owned());
        }
    }
    for page_id in &children {
        validate_hash(page_id, "reachable state page ID")?;
    }
    Ok(children)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedStateDelta {
    pub format_version: u32,
    pub source_root: String,
    pub target_root: String,
    pub new_pages: Vec<StatePageBlob>,
    pub token_hash: String,
}

impl PreparedStateDelta {
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != STATE_PAGE_FORMAT_VERSION {
            return Err(page_error(
                "prepared state delta uses an unsupported format",
            ));
        }
        validate_hash(&self.source_root, "state delta source root")?;
        validate_hash(&self.target_root, "state delta target root")?;
        validate_hash(&self.token_hash, "state delta token")?;
        if self.new_pages.len() > MAX_STATE_DELTA_PAGES {
            return Err(page_error("prepared state delta contains too many pages"));
        }
        let mut ids = BTreeSet::new();
        for page in &self.new_pages {
            page.validate()?;
            if !ids.insert(page.page_id.clone()) {
                return Err(page_error("prepared state delta contains duplicate pages"));
            }
        }
        let expected =
            canonical_hash_for_delta(&self.source_root, &self.target_root, &self.new_pages);
        if self.token_hash != expected {
            return Err(page_error("prepared state delta token is inconsistent"));
        }
        Ok(())
    }
}

pub fn prepare_state_delta(
    source_root: &str,
    target_root: &str,
    pages: Vec<StatePageBlob>,
) -> Result<PreparedStateDelta, CanwuError> {
    validate_hash(source_root, "state delta source root")?;
    validate_hash(target_root, "state delta target root")?;
    let prepared = PreparedStateDelta {
        format_version: STATE_PAGE_FORMAT_VERSION,
        source_root: source_root.to_owned(),
        target_root: target_root.to_owned(),
        new_pages: pages,
        token_hash: String::new(),
    };
    let token_hash = canonical_hash_for_delta(
        &prepared.source_root,
        &prepared.target_root,
        &prepared.new_pages,
    );
    let prepared = PreparedStateDelta {
        token_hash,
        ..prepared
    };
    prepared.validate()?;
    Ok(prepared)
}

pub fn verify_state_delta(
    prepared: &PreparedStateDelta,
    provider: &dyn StatePageProvider,
) -> Result<(), CanwuError> {
    prepared.validate()?;
    for page in &prepared.new_pages {
        let loaded = provider.load_state_page(&page.page_id)?.ok_or_else(|| {
            CanwuError::new(ErrorCode::StatePageUnavailable, "state page is unavailable")
        })?;
        loaded.validate()?;
        if loaded != *page {
            return Err(page_error(
                "provider returned bytes different from the prepared page",
            ));
        }
    }
    Ok(())
}

fn canonical_hash_for_delta(
    source_root: &str,
    target_root: &str,
    pages: &[StatePageBlob],
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(source_root.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(target_root.as_bytes());
    bytes.push(0);
    for page in pages {
        bytes.extend_from_slice(page.page_id.as_bytes());
        bytes.push(0);
    }
    canonical_byte_hash("canwu.state-delta.v1", &bytes)
}

fn validate_hash(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(page_error(format!(
            "{label} must be a lower-case 32-byte hash"
        )));
    }
    Ok(())
}

fn page_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidArchive, message)
}
