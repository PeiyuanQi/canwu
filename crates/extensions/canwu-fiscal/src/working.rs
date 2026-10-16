//! The fiscal records one boundary or one command reads and writes.
//!
//! A [`Working`] set loads records from a [`RecordSource`] when a handler
//! first needs them, keeps them in a partial [`FiscalState`] that holds only
//! what was loaded, and proposes a write of exactly the records that changed.
//! Handlers that need one field of a record read it from the record's payload
//! without decoding the record ([`LedgerHead`], [`TargetHead`], the binding
//! directory, the index pages).

use crate::model::{
    CompiledFiscalCatalog, FISCAL_REPORT_QUEUE_CYCLES, FISCAL_REPORT_QUEUE_PARTS,
    FiscalActionTarget, FiscalMechanism, FiscalOffice, FiscalOfficeObserver, FiscalOutcomeRing,
    FiscalReportQueueEntry, FiscalReportSchedule, FiscalState, MAX_FISCAL_REPORT_CYCLE_SLOTS,
    fiscal_office_key, invalid, office_binding_ids,
};
use crate::records::{
    FiscalArchiveRecord, FiscalDirectoryBucket, FiscalDirectoryRecord, FiscalIndexOwner,
    FiscalIndexRecord, FiscalJurisdictionRecord, FiscalLedgerRecord, FiscalObserverRecord,
    FiscalOfficeRecord, FiscalReportQueuePage, FiscalRootRecord, archive_draft, directory_bucket,
    directory_draft, directory_reference, fiscal_archive_reference, fiscal_jurisdiction_reference,
    fiscal_ledger_reference, fiscal_observer_reference, fiscal_report_schedule_reference,
    fiscal_root_reference, index_reference, jurisdiction_draft, ledger_draft, observer_draft,
    office_draft, office_reference, queue_draft, queue_reference, report_queue_page, root_draft,
    schedule_draft,
};
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordDraft, DomainRecordMutation, DomainRecordRef,
    DomainRecordType, EntityRef, SimTime, SimulationView, TypedDomainRecordRef,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Where fiscal records are read from: a boundary's or a command's view, or
/// a run, for host reads.
pub(crate) trait RecordSource {
    fn fiscal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError>;
}

impl RecordSource for SimulationView<'_> {
    fn fiscal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        self.domain_record(reference)
    }
}

/// A host run, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`], as a
/// source of fiscal records for host reads.
pub(crate) struct HostRecords<'a, H: canwu_api::CanwuRun + ?Sized>(pub(crate) &'a H);

impl<H: canwu_api::CanwuRun + ?Sized> RecordSource for HostRecords<'_, H> {
    fn fiscal_record(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<&DomainRecord>, CanwuError> {
        Ok(self.0.domain_record(reference))
    }
}

/// Reads one active record, or `None`.
pub(crate) fn active_record<'s, S: RecordSource + ?Sized, T: DomainRecordType>(
    source: &'s S,
    reference: &TypedDomainRecordRef<T>,
) -> Result<Option<&'s DomainRecord>, CanwuError> {
    Ok(source
        .fiscal_record(reference.as_untyped())?
        .filter(|record| record.lifecycle == canwu_api::DomainRecordLifecycle::Active))
}

/// Decodes one field of a record's payload, or its default when it is
/// missing.
pub(crate) fn field<T: DeserializeOwned + Default>(
    payload: &Value,
    name: &str,
) -> Result<T, CanwuError> {
    payload
        .get(name)
        .map(|value| {
            T::deserialize(value).map_err(|error| {
                invalid(format!(
                    "fiscal record field {name} could not be decoded: {error}"
                ))
            })
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

/// What admission reads of a ledger without decoding it: its scope.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct LedgerHead {
    pub(crate) scope: ScopeHead,
}

/// What admission reads of a scope binding.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ScopeHead {
    pub(crate) institution: EntityRef,
    pub(crate) jurisdiction_id: String,
}

/// The revision and recent outcomes of an action target.
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct TargetHead {
    pub(crate) revision: u64,
    #[serde(default)]
    pub(crate) outcomes: FiscalOutcomeRing,
}

/// Which of a working set's records an action's copy holds. Offices in
/// `cycle_offices` are copied with only their cycle bookkeeping and
/// watermark, which is all a scope action changes of them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Touch {
    pub(crate) ledgers: BTreeSet<String>,
    pub(crate) offices: BTreeSet<String>,
    pub(crate) cycle_offices: BTreeSet<String>,
    pub(crate) jurisdictions: BTreeSet<String>,
    pub(crate) observers: BTreeSet<String>,
    pub(crate) archive: bool,
}

/// The records a boundary or a command has loaded, as they stand now.
pub(crate) struct Working<'a, S: RecordSource + ?Sized> {
    source: &'a S,
    /// The loaded records. Maps hold only what was loaded; the root is
    /// always loaded, the archive head only when `archive_loaded`.
    pub(crate) state: FiscalState,
    archive_loaded: bool,
    /// The revision of every action target when it was loaded: its revision
    /// at the start of the boundary, since only loaded records change.
    pub(crate) entry_revisions: BTreeMap<FiscalActionTarget, u64>,
    /// The book revisions of each loaded ledger, by cycle, when it was
    /// loaded.
    loaded_books: BTreeMap<String, BTreeMap<String, u64>>,
    /// The binding and proposal IDs of each loaded office when it was
    /// loaded.
    loaded_ids: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>,
    /// Records that changed since they were loaded.
    dirty: BTreeSet<DomainRecordRef>,
    /// The version of every loaded record that exists in the source.
    versions: BTreeMap<DomainRecordRef, u64>,
    /// Report-queue pages loaded to be changed.
    queue: BTreeMap<String, FiscalReportQueuePage>,
    /// Each loaded report-queue page as it was loaded, so that only a page
    /// that changed is written.
    queue_loaded: BTreeMap<String, FiscalReportQueuePage>,
    /// The report schedule as it was loaded, once it is; the state holds it
    /// as it stands now.
    schedule_loaded: Option<FiscalReportSchedule>,
    /// Index pages read, by owner and page number: each is decoded once per
    /// working set, and found again without naming its record.
    index: BTreeMap<FiscalIndexOwner, BTreeMap<u32, Option<Vec<String>>>>,
}

impl<'a, S: RecordSource + ?Sized> Working<'a, S> {
    /// A working set over `source` with its root loaded, or `None` when the
    /// run has no fiscal root.
    pub(crate) fn new(source: &'a S) -> Result<Option<Self>, CanwuError> {
        let Some(record) = active_record(source, &fiscal_root_reference())? else {
            return Ok(None);
        };
        let root = record.decode_payload::<FiscalRootRecord>()?;
        let mut versions = BTreeMap::new();
        versions.insert(record.reference.clone(), record.version);
        Ok(Some(Self {
            source,
            state: FiscalState {
                root,
                archive: crate::FiscalArchiveHeadV1::default(),
                offices: BTreeMap::new(),
                ledgers: BTreeMap::new(),
                jurisdictions: BTreeMap::new(),
                observers: BTreeMap::new(),
                report_queue: BTreeMap::new(),
                report_schedule: FiscalReportSchedule::default(),
            },
            archive_loaded: false,
            entry_revisions: BTreeMap::new(),
            loaded_books: BTreeMap::new(),
            loaded_ids: BTreeMap::new(),
            dirty: BTreeSet::new(),
            versions,
            queue: BTreeMap::new(),
            queue_loaded: BTreeMap::new(),
            schedule_loaded: None,
            index: BTreeMap::new(),
        }))
    }

    pub(crate) fn source(&self) -> &'a S {
        self.source
    }

    fn load<T: DomainRecordType>(
        &mut self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Result<Option<T::Payload>, CanwuError>
    where
        T::Payload: DeserializeOwned,
    {
        let Some(record) = active_record(self.source, reference)? else {
            return Ok(None);
        };
        self.versions
            .insert(record.reference.clone(), record.version);
        Ok(Some(record.decode_payload::<T>()?))
    }

    /// Loads the ledger of a scope; `false` when there is none.
    pub(crate) fn ensure_ledger(&mut self, scope: &str) -> Result<bool, CanwuError> {
        if self.state.ledgers.contains_key(scope) {
            return Ok(true);
        }
        let Some(ledger) = self.load(&fiscal_ledger_reference(scope))? else {
            return Ok(false);
        };
        if ledger.scope.id != scope {
            return Err(invalid("a fiscal ledger is not keyed by its scope"));
        }
        self.entry_revisions.insert(
            FiscalActionTarget::Ledger(scope.to_owned()),
            ledger.revision,
        );
        self.loaded_books.insert(
            scope.to_owned(),
            ledger
                .books
                .iter()
                .map(|(cycle, book)| (cycle.clone(), book.revision))
                .collect(),
        );
        self.state.ledgers.insert(scope.to_owned(), ledger);
        Ok(true)
    }

    /// Loads an office by its key; `false` when there is none.
    pub(crate) fn ensure_office_key(&mut self, key: &str) -> Result<bool, CanwuError> {
        if self.state.offices.contains_key(key) {
            return Ok(true);
        }
        let Some(office) = self.load(&office_reference(key))? else {
            return Ok(false);
        };
        if fiscal_office_key(&office.institution) != key {
            return Err(invalid("a fiscal office is not keyed by its institution"));
        }
        self.entry_revisions.insert(
            FiscalActionTarget::Office(office.institution.clone()),
            office.revision,
        );
        self.loaded_ids.insert(key.to_owned(), office_ids(&office));
        self.state.offices.insert(key.to_owned(), office);
        Ok(true)
    }

    /// Loads the office of an institution; `false` when it has none.
    pub(crate) fn ensure_office(&mut self, institution: &EntityRef) -> Result<bool, CanwuError> {
        self.ensure_office_key(&fiscal_office_key(institution))
    }

    /// Loads the jurisdiction of a region; `false` when it has none.
    pub(crate) fn ensure_jurisdiction(&mut self, region: &str) -> Result<bool, CanwuError> {
        if self.state.jurisdictions.contains_key(region) {
            return Ok(true);
        }
        let Some(jurisdiction) = self.load(&fiscal_jurisdiction_reference(region))? else {
            return Ok(false);
        };
        self.entry_revisions.insert(
            FiscalActionTarget::Jurisdiction(region.to_owned()),
            jurisdiction.revision,
        );
        self.state
            .jurisdictions
            .insert(region.to_owned(), jurisdiction);
        Ok(true)
    }

    /// Loads an observer; `false` when there is none.
    pub(crate) fn ensure_observer(&mut self, observer: &str) -> Result<bool, CanwuError> {
        if self.state.observers.contains_key(observer) {
            return Ok(true);
        }
        let Some(record) = self.load(&fiscal_observer_reference(observer))? else {
            return Ok(false);
        };
        self.entry_revisions.insert(
            FiscalActionTarget::Observer(observer.to_owned()),
            record.revision,
        );
        self.state.observers.insert(observer.to_owned(), record);
        Ok(true)
    }

    /// Loads the record of an action target; `false` when there is none.
    pub(crate) fn ensure_target(
        &mut self,
        target: &FiscalActionTarget,
    ) -> Result<bool, CanwuError> {
        match target {
            FiscalActionTarget::Ledger(scope) => self.ensure_ledger(scope),
            FiscalActionTarget::Office(institution) => self.ensure_office(institution),
            FiscalActionTarget::Jurisdiction(region) => self.ensure_jurisdiction(region),
            FiscalActionTarget::Observer(observer) => self.ensure_observer(observer),
        }
    }

    /// Loads the archive head.
    pub(crate) fn ensure_archive(&mut self) -> Result<(), CanwuError> {
        if !self.archive_loaded {
            self.state.archive = self
                .load(&fiscal_archive_reference())?
                .ok_or_else(|| invalid("the fiscal archive record is missing"))?;
            self.archive_loaded = true;
        }
        Ok(())
    }

    /// Loads the report schedule into the state, once.
    pub(crate) fn ensure_schedule(&mut self) -> Result<(), CanwuError> {
        if self.schedule_loaded.is_none() {
            let schedule = self
                .load(&fiscal_report_schedule_reference())?
                .ok_or_else(|| invalid("the fiscal report schedule record is missing"))?;
            self.state.report_schedule = schedule.clone();
            self.schedule_loaded = Some(schedule);
        }
        Ok(())
    }

    /// The office the binding directory names for a binding or policy ID.
    pub(crate) fn binding_office(&self, id: &str) -> Result<Option<String>, CanwuError> {
        self.directory_entry(id, "bindings")
    }

    /// The office the binding directory names for a proposal ID.
    pub(crate) fn proposal_office(&self, id: &str) -> Result<Option<String>, CanwuError> {
        self.directory_entry(id, "proposals")
    }

    fn directory_entry(&self, id: &str, namespace: &str) -> Result<Option<String>, CanwuError> {
        let Some(record) = active_record(self.source, &directory_reference(&directory_bucket(id)))?
        else {
            return Ok(None);
        };
        Ok(record
            .payload
            .get(namespace)
            .and_then(|entries| entries.get(id))
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    /// Loads the office that holds a binding or policy ID, and returns its
    /// key.
    pub(crate) fn ensure_binding_office(&mut self, id: &str) -> Result<Option<String>, CanwuError> {
        let Some(key) = self.binding_office(id)? else {
            return Ok(None);
        };
        Ok(self.ensure_office_key(&key)?.then_some(key))
    }

    /// Loads the office that holds a proposal ID, and returns its key.
    pub(crate) fn ensure_proposal_office(
        &mut self,
        id: &str,
    ) -> Result<Option<String>, CanwuError> {
        let Some(key) = self.proposal_office(id)? else {
            return Ok(None);
        };
        Ok(self.ensure_office_key(&key)?.then_some(key))
    }

    /// The scopes one index lists, every page of it.
    pub(crate) fn index_scopes(
        &mut self,
        owner: &FiscalIndexOwner,
    ) -> Result<Vec<String>, CanwuError> {
        let mut scopes = Vec::new();
        for page in 0_u32.. {
            match self.index_page(owner, page)? {
                Some(listed) => scopes.extend(listed.iter().cloned()),
                None => break,
            }
        }
        Ok(scopes)
    }

    /// The `rank`-th scope one index lists.
    pub(crate) fn index_scope(
        &mut self,
        owner: &FiscalIndexOwner,
        rank: u32,
    ) -> Result<Option<String>, CanwuError> {
        let (page, place) = FiscalIndexOwner::locate(rank);
        Ok(self
            .index_page(owner, page)?
            .and_then(|scopes| scopes.get(place).cloned()))
    }

    /// One page of an index. A page is read and decoded once per working
    /// set; a later read finds it by owner and number, without naming its
    /// record, so a run of rows of one office costs one decode per page.
    pub(crate) fn index_page(
        &mut self,
        owner: &FiscalIndexOwner,
        page: u32,
    ) -> Result<Option<&Vec<String>>, CanwuError> {
        let cached = self
            .index
            .get(owner)
            .is_some_and(|pages| pages.contains_key(&page));
        if !cached {
            let id = owner.page_id(page);
            let scopes = match active_record(self.source, &index_reference(&id))? {
                Some(record) => {
                    let decoded = record.decode_payload::<FiscalIndexRecord>()?;
                    if decoded.owner != *owner || decoded.page != page {
                        return Err(invalid("a fiscal index page names another owner"));
                    }
                    Some(decoded.scopes)
                }
                None => None,
            };
            self.index
                .entry(owner.clone())
                .or_default()
                .insert(page, scopes);
        }
        Ok(self
            .index
            .get(owner)
            .and_then(|pages| pages.get(&page))
            .and_then(Option::as_ref))
    }

    /// What admission reads of a scope's ledger, without decoding it.
    pub(crate) fn ledger_head(&self, scope: &str) -> Result<Option<LedgerHead>, CanwuError> {
        if let Some(ledger) = self.state.ledgers.get(scope) {
            return Ok(Some(LedgerHead {
                scope: ScopeHead {
                    institution: ledger.scope.institution.clone(),
                    jurisdiction_id: ledger.scope.jurisdiction_id.clone(),
                },
            }));
        }
        active_record(self.source, &fiscal_ledger_reference(scope))?
            .map(|record| {
                LedgerHead::deserialize(&record.payload)
                    .map_err(|error| invalid(format!("fiscal ledger is invalid: {error}")))
            })
            .transpose()
    }

    /// Whether a scope's hot books hold an object of this ID among the
    /// objects `kinds` names (`assessments`, `execution_requests`,
    /// `execution_receipts`), read without decoding the ledger.
    pub(crate) fn ledger_holds(
        &self,
        scope: &str,
        id: &str,
        kinds: &[&str],
    ) -> Result<bool, CanwuError> {
        if let Some(ledger) = self.state.ledgers.get(scope) {
            return Ok(ledger.books.values().any(|book| {
                kinds.iter().any(|kind| match *kind {
                    "assessments" => book.assessments.contains_key(id),
                    "execution_requests" => book.execution_requests.contains_key(id),
                    "execution_receipts" => book.execution_receipts.contains_key(id),
                    _ => false,
                })
            }));
        }
        let Some(record) = active_record(self.source, &fiscal_ledger_reference(scope))? else {
            return Ok(false);
        };
        Ok(record
            .payload
            .get("books")
            .and_then(Value::as_object)
            .is_some_and(|books| {
                books.values().any(|book| {
                    kinds
                        .iter()
                        .any(|kind| book.get(*kind).and_then(|map| map.get(id)).is_some())
                })
            }))
    }

    /// The revision and recent outcomes of an action target, read without
    /// decoding it.
    pub(crate) fn target_head(
        &self,
        target: &FiscalActionTarget,
    ) -> Result<Option<TargetHead>, CanwuError> {
        if let (Some(revision), Some(ring)) = (
            self.state.target_revision(target),
            self.state.outcome_ring(target),
        ) {
            return Ok(Some(TargetHead {
                revision,
                outcomes: ring.clone(),
            }));
        }
        let record = match target {
            FiscalActionTarget::Ledger(scope) => {
                active_record(self.source, &fiscal_ledger_reference(scope))?
            }
            FiscalActionTarget::Office(institution) => active_record(
                self.source,
                &office_reference(&fiscal_office_key(institution)),
            )?,
            FiscalActionTarget::Jurisdiction(region) => {
                active_record(self.source, &fiscal_jurisdiction_reference(region))?
            }
            FiscalActionTarget::Observer(observer) => {
                active_record(self.source, &fiscal_observer_reference(observer))?
            }
        };
        record
            .map(|record| {
                TargetHead::deserialize(&record.payload)
                    .map_err(|error| invalid(format!("fiscal record is invalid: {error}")))
            })
            .transpose()
    }

    /// The observers of an office, each with the first row of its
    /// institution in its report, its granularity, and whether it is on an
    /// interval, and the
    /// office's mechanisms, read without decoding the office.
    pub(crate) fn office_observers(
        &self,
        key: &str,
    ) -> Result<
        (
            BTreeMap<String, FiscalOfficeObserver>,
            BTreeSet<FiscalMechanism>,
        ),
        CanwuError,
    > {
        if let Some(office) = self.state.offices.get(key) {
            return Ok((office.observers.clone(), office.mechanisms.clone()));
        }
        match active_record(self.source, &office_reference(key))? {
            Some(record) => Ok((
                field(&record.payload, "observers")?,
                field(&record.payload, "mechanisms")?,
            )),
            None => Ok((BTreeMap::new(), BTreeSet::new())),
        }
    }

    /// The hot cycles of an office, read without decoding it.
    pub(crate) fn office_cycles(&self, key: &str) -> Result<BTreeSet<String>, CanwuError> {
        if let Some(office) = self.state.offices.get(key) {
            return Ok(office.cycles.keys().cloned().collect());
        }
        Ok(match active_record(self.source, &office_reference(key))? {
            Some(record) => record
                .payload
                .get("cycles")
                .and_then(Value::as_object)
                .map(|cycles| cycles.keys().cloned().collect())
                .unwrap_or_default(),
            None => BTreeSet::new(),
        })
    }

    /// Marks a loaded record changed.
    pub(crate) fn touch(&mut self, reference: DomainRecordRef) {
        self.dirty.insert(reference);
    }

    /// Marks the target of an action changed.
    pub(crate) fn touch_target(&mut self, target: &FiscalActionTarget) {
        let reference = match target {
            FiscalActionTarget::Ledger(scope) => fiscal_ledger_reference(scope).into_untyped(),
            FiscalActionTarget::Office(institution) => {
                office_reference(&fiscal_office_key(institution)).into_untyped()
            }
            FiscalActionTarget::Jurisdiction(region) => {
                fiscal_jurisdiction_reference(region).into_untyped()
            }
            FiscalActionTarget::Observer(observer) => {
                fiscal_observer_reference(observer).into_untyped()
            }
        };
        self.touch(reference);
    }

    /// The scopes whose ledgers this working set has changed.
    pub(crate) fn changed_ledgers(&self) -> impl Iterator<Item = &String> {
        self.dirty.iter().filter_map(|reference| {
            reference
                .kind
                .matches_type::<FiscalLedgerRecord>()
                .then_some(&reference.id)
        })
    }

    /// A copy of the records `touch` names, for one action to change. The
    /// report queue and schedule, which no action reads, are left out.
    pub(crate) fn copy(&self, touch: &Touch) -> FiscalState {
        fn pick(keys: &BTreeSet<String>) -> impl Iterator<Item = String> + '_ {
            keys.iter().cloned()
        }
        FiscalState {
            root: self.state.root.clone(),
            archive: if touch.archive {
                self.state.archive.clone()
            } else {
                crate::FiscalArchiveHeadV1::default()
            },
            offices: pick(&touch.offices)
                .filter_map(|key| {
                    self.state
                        .offices
                        .get(&key)
                        .map(|office| (key, office.clone()))
                })
                .chain(
                    pick(&touch.cycle_offices)
                        .filter(|key| !touch.offices.contains(key))
                        .filter_map(|key| {
                            self.state
                                .offices
                                .get(&key)
                                .map(|office| (key, cycle_office(office)))
                        }),
                )
                .collect(),
            ledgers: pick(&touch.ledgers)
                .filter_map(|key| {
                    self.state
                        .ledgers
                        .get(&key)
                        .map(|ledger| (key, ledger.clone()))
                })
                .collect(),
            jurisdictions: pick(&touch.jurisdictions)
                .filter_map(|key| {
                    self.state
                        .jurisdictions
                        .get(&key)
                        .map(|jurisdiction| (key, jurisdiction.clone()))
                })
                .collect(),
            observers: pick(&touch.observers)
                .filter_map(|key| {
                    self.state
                        .observers
                        .get(&key)
                        .map(|observer| (key, observer.clone()))
                })
                .collect(),
            report_queue: BTreeMap::new(),
            report_schedule: FiscalReportSchedule::default(),
        }
    }

    /// Takes back a copy an action changed, marking every record that
    /// differs, and queues the report work the copy holds, as the binding it
    /// is for is scheduled after the action.
    pub(crate) fn absorb(&mut self, touch: &Touch, copy: FiscalState) -> Result<(), CanwuError> {
        let FiscalState {
            archive,
            offices,
            ledgers,
            jurisdictions,
            observers,
            report_queue,
            ..
        } = copy;
        if touch.archive && archive != self.state.archive {
            self.state.archive = archive;
            self.touch(fiscal_archive_reference().into_untyped());
        }
        for (key, office) in offices {
            let Some(current) = self.state.offices.get_mut(&key) else {
                continue;
            };
            if !touch.offices.contains(&key) {
                if current.cycles != office.cycles || current.watermark != office.watermark {
                    current.cycles = office.cycles;
                    current.watermark = office.watermark;
                    self.dirty.insert(office_reference(&key).into_untyped());
                }
            } else if *current != office {
                *current = office;
                self.dirty.insert(office_reference(&key).into_untyped());
            }
        }
        for (key, ledger) in ledgers {
            if self.state.ledgers.get(&key) != Some(&ledger) {
                self.dirty
                    .insert(fiscal_ledger_reference(&key).into_untyped());
                self.state.ledgers.insert(key, ledger);
            }
        }
        for (key, jurisdiction) in jurisdictions {
            if self.state.jurisdictions.get(&key) != Some(&jurisdiction) {
                self.dirty
                    .insert(fiscal_jurisdiction_reference(&key).into_untyped());
                self.state.jurisdictions.insert(key, jurisdiction);
            }
        }
        let mut rebound = BTreeSet::new();
        for (key, observer) in observers {
            if self.state.observers.get(&key) != Some(&observer) {
                if self
                    .state
                    .observers
                    .get(&key)
                    .is_some_and(|before| before.binding != observer.binding)
                {
                    rebound.insert(key.clone());
                }
                self.dirty
                    .insert(fiscal_observer_reference(&key).into_untyped());
                self.state.observers.insert(key, observer);
            }
        }
        for (observer, entry) in report_queue {
            let interval = self
                .state
                .observers
                .get(&observer)
                .is_some_and(|observer| observer.binding.scheduled_cadence().is_interval());
            self.queue(&observer, entry, interval)?;
            // A binding re-pointed or removed has a new holder, or none, so
            // the holder its entry was parked for no longer matters: the
            // report system checks the binding again when the entry is due.
            if rebound.contains(&observer)
                && let Some(queued) = self
                    .queue_page(&report_queue_page(&observer))?
                    .entries
                    .get_mut(&observer)
            {
                queued.parked = false;
            }
        }
        Ok(())
    }

    /// Adds report work owed to one observer to its queue page. Work for an
    /// observer scheduled on change is due at the earlier of its own time and
    /// what the observer's entry is due at already. Work for an observer on
    /// an interval (`interval`) joins the entry an active interval binding
    /// always has, whose due time does not move; an interval binding without
    /// one has been removed, is never published to again, and is owed
    /// nothing.
    pub(crate) fn queue(
        &mut self,
        observer: &str,
        entry: FiscalReportQueueEntry,
        interval: bool,
    ) -> Result<(), CanwuError> {
        let page = self.queue_page(&report_queue_page(observer))?;
        if let Some(existing) = page.entries.get_mut(observer) {
            merge_queue_entry(existing, entry, interval);
        } else if !interval {
            let mut entry = entry;
            bound_queue_entry(&mut entry);
            page.entries.insert(observer.to_owned(), entry);
        }
        Ok(())
    }

    /// Loads a report-queue page to change it.
    pub(crate) fn queue_page(
        &mut self,
        page: &str,
    ) -> Result<&mut FiscalReportQueuePage, CanwuError> {
        if !self.queue.contains_key(page) {
            let loaded = self.load(&queue_reference(page))?.unwrap_or_default();
            self.queue_loaded.insert(page.to_owned(), loaded.clone());
            self.queue.insert(page.to_owned(), loaded);
        }
        self.queue
            .get_mut(page)
            .ok_or_else(|| invalid("fiscal report-queue page is unavailable"))
    }

    /// Queues the report parts that the change of each changed ledger's
    /// books touches, for every observer of the ledger's office: one part of
    /// each changed cycle, the one that holds the scope's row, or, at
    /// institution granularity, the row of the scope's mechanism. Each
    /// office's observer index is read once, and each observer gets one
    /// entry for all of its office's changes, due as its cadence schedules
    /// it; no observer record is read.
    pub(crate) fn queue_changed_books(&mut self, at: SimTime) -> Result<(), CanwuError> {
        type Change = (u32, FiscalMechanism, Vec<String>);
        let mut owed = BTreeMap::<String, Vec<Change>>::new();
        for scope in self.changed_ledgers() {
            let Some(ledger) = self.state.ledgers.get(scope) else {
                continue;
            };
            let before = self.loaded_books.get(scope);
            let mut cycles: BTreeSet<&String> = ledger
                .books
                .iter()
                .filter(|(cycle, book)| {
                    before.and_then(|books| books.get(*cycle)) != Some(&book.revision)
                })
                .map(|(cycle, _)| cycle)
                .collect();
            if let Some(before) = before {
                cycles.extend(
                    before
                        .keys()
                        .filter(|cycle| !ledger.books.contains_key(*cycle)),
                );
            }
            if cycles.is_empty() {
                continue;
            }
            owed.entry(ledger.office.clone()).or_default().push((
                ledger.rank,
                ledger.scope.mechanism,
                cycles.into_iter().cloned().collect::<Vec<_>>(),
            ));
        }
        for (office, changes) in owed {
            let (observers, mechanisms) = self.office_observers(&office)?;
            for (observer, listed) in observers {
                let mut cycles = BTreeMap::<String, BTreeSet<u32>>::new();
                for (rank, mechanism, changed) in &changes {
                    let part =
                        crate::derive::part_of_row(listed.row(*rank, *mechanism, &mechanisms));
                    for cycle in changed {
                        cycles.entry(cycle.clone()).or_default().insert(part);
                    }
                }
                self.queue(
                    &observer,
                    FiscalReportQueueEntry {
                        due_at: at,
                        cycles,
                        ..FiscalReportQueueEntry::default()
                    },
                    listed.interval,
                )?;
            }
        }
        Ok(())
    }

    /// The directory changes the changed offices make: each binding, policy,
    /// or proposal ID an office gained points to it, and each it lost no
    /// longer does. Each changed bucket comes with its version, when it
    /// exists.
    #[allow(clippy::type_complexity)]
    fn directory_changes(
        &self,
    ) -> Result<BTreeMap<String, (Option<u64>, FiscalDirectoryBucket)>, CanwuError> {
        // Every ID an office lost or gained, with whether it is a proposal
        // ID. An ID one office drops and another takes in the same boundary
        // moves, so every removal is applied before any insertion.
        let mut lost = Vec::new();
        let mut gained = Vec::new();
        for (key, office) in &self.state.offices {
            if !self.dirty.contains(office_reference(key).as_untyped()) {
                continue;
            }
            let (bindings, proposals) = office_ids(office);
            let (before_bindings, before_proposals) =
                self.loaded_ids.get(key).cloned().unwrap_or_default();
            for (now, before, proposal) in [
                (bindings, before_bindings, false),
                (proposals, before_proposals, true),
            ] {
                lost.extend(
                    before
                        .difference(&now)
                        .map(|id| (key.clone(), id.clone(), proposal)),
                );
                gained.extend(
                    now.difference(&before)
                        .map(|id| (key.clone(), id.clone(), proposal)),
                );
            }
        }
        let mut buckets = BTreeMap::<String, (Option<u64>, FiscalDirectoryBucket)>::new();
        for (removal, (key, id, proposal)) in lost
            .iter()
            .map(|change| (true, change))
            .chain(gained.iter().map(|change| (false, change)))
        {
            let bucket = directory_bucket(id);
            if !buckets.contains_key(&bucket) {
                let loaded = match active_record(self.source, &directory_reference(&bucket))? {
                    Some(record) => (
                        Some(record.version),
                        record.decode_payload::<FiscalDirectoryRecord>()?,
                    ),
                    None => (None, FiscalDirectoryBucket::default()),
                };
                buckets.insert(bucket.clone(), loaded);
            }
            let (_, entries) = buckets
                .get_mut(&bucket)
                .ok_or_else(|| invalid("fiscal directory bucket is unavailable"))?;
            let map = if *proposal {
                &mut entries.proposals
            } else {
                &mut entries.bindings
            };
            if removal {
                if map.get(id) == Some(key) {
                    map.remove(id);
                }
            } else {
                if map.get(id).is_some_and(|holder| holder != key) {
                    return Err(invalid(format!(
                        "fiscal binding or proposal ID {id} is held by another office"
                    )));
                }
                map.insert(id.clone(), key.clone());
            }
        }
        Ok(buckets)
    }

    /// Brings the report schedule and the root up to date with the queue
    /// pages that changed: the schedule keeps each page's earliest due time
    /// and parked count, and the root the least due time and whether any
    /// entry is parked. Each is marked changed only when it differs from
    /// what was loaded.
    fn update_schedule(&mut self) -> Result<(), CanwuError> {
        let pages_changed = self
            .queue
            .iter()
            .any(|(page, entries)| self.queue_loaded.get(page) != Some(entries));
        if pages_changed {
            self.ensure_schedule()?;
        }
        let Some(loaded) = &self.schedule_loaded else {
            return Ok(());
        };
        let mut schedule = self.state.report_schedule.clone();
        for (page, entries) in &self.queue {
            if self.queue_loaded.get(page) != Some(entries) {
                schedule.update_page(page, entries.entries.values());
            }
        }
        if schedule != *loaded {
            crate::model::validate_report_schedule(&schedule)?;
            self.dirty
                .insert(fiscal_report_schedule_reference().into_untyped());
        }
        let (earliest_due, parked) = schedule.root_fields();
        let cursor = if parked {
            self.state.root.parked_cursor.clone()
        } else {
            None
        };
        if (earliest_due, parked, &cursor)
            != (
                self.state.root.earliest_due,
                self.state.root.parked,
                &self.state.root.parked_cursor,
            )
        {
            self.state.root.earliest_due = earliest_due;
            self.state.root.parked = parked;
            self.state.root.parked_cursor = cursor;
            self.dirty.insert(fiscal_root_reference().into_untyped());
        }
        self.state.report_schedule = schedule;
        Ok(())
    }

    /// The writes of every changed record, each validated on its own, and
    /// of the directory buckets, queue pages, schedule, and root they need.
    pub(crate) fn directives(
        mut self,
        catalog: &CompiledFiscalCatalog,
        summary: &str,
    ) -> Result<Vec<canwu_api::BoundaryDirective>, CanwuError> {
        let directory = self.directory_changes()?;
        self.update_schedule()?;
        let mut drafts: Vec<DomainRecordDraft> = Vec::new();
        for reference in &self.dirty {
            let draft = if reference.kind.matches_type::<FiscalRootRecord>() {
                crate::model::validate_root(&self.state.root, catalog)?;
                root_draft(&self.state.root)?
            } else if reference.kind.matches_type::<FiscalArchiveRecord>() {
                self.state.archive.validate()?;
                archive_draft(&self.state.archive)?
            } else if reference
                .kind
                .matches_type::<crate::records::FiscalReportScheduleRecord>()
            {
                schedule_draft(&self.state.report_schedule)?
            } else if reference.kind.matches_type::<FiscalOfficeRecord>() {
                let office = self
                    .state
                    .offices
                    .get(&reference.id)
                    .ok_or_else(|| invalid("a changed fiscal office is not loaded"))?;
                crate::model::validate_office(office)?;
                office_draft(office)?
            } else if reference.kind.matches_type::<FiscalLedgerRecord>() {
                let ledger = self
                    .state
                    .ledgers
                    .get(&reference.id)
                    .ok_or_else(|| invalid("a changed fiscal ledger is not loaded"))?;
                crate::model::validate_ledger(ledger, catalog)?;
                ledger_draft(ledger)?
            } else if reference.kind.matches_type::<FiscalJurisdictionRecord>() {
                let jurisdiction = self
                    .state
                    .jurisdictions
                    .get(&reference.id)
                    .ok_or_else(|| invalid("a changed fiscal jurisdiction is not loaded"))?;
                crate::model::validate_jurisdiction(jurisdiction, catalog)?;
                jurisdiction_draft(jurisdiction)?
            } else if reference.kind.matches_type::<FiscalObserverRecord>() {
                let observer = self
                    .state
                    .observers
                    .get(&reference.id)
                    .ok_or_else(|| invalid("a changed fiscal observer is not loaded"))?;
                crate::model::validate_observer(observer)?;
                observer_draft(observer)?
            } else {
                return Err(invalid("a changed fiscal record has no writer"));
            };
            drafts.push(draft);
        }
        for (bucket, (version, entries)) in directory {
            let draft = directory_draft(&bucket, &entries)?;
            if let Some(version) = version {
                self.versions.insert(draft.reference.clone(), version);
            }
            drafts.push(draft);
        }
        for (page, entries) in &self.queue {
            if self.queue_loaded.get(page) == Some(entries) {
                continue;
            }
            for entry in entries.entries.values() {
                crate::model::validate_queue_entry(entry)?;
            }
            let reference = queue_reference(page).into_untyped();
            if entries.entries.is_empty() && !self.versions.contains_key(&reference) {
                continue;
            }
            drafts.push(queue_draft(page, entries)?);
        }
        let mut directives = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let mutation = match self.versions.get(&draft.reference).copied() {
                Some(expected_version) => DomainRecordMutation::Update {
                    record: draft,
                    expected_version,
                },
                None => DomainRecordMutation::Create { record: draft },
            };
            directives.push(canwu_api::BoundaryDirective::MutateRecord {
                mutation,
                summary: summary.to_owned(),
            });
        }
        Ok(directives)
    }
}

/// The binding and policy IDs, and the proposal IDs, an office holds.
fn office_ids(office: &FiscalOffice) -> (BTreeSet<String>, BTreeSet<String>) {
    (
        office_binding_ids(office).cloned().collect(),
        office.audit_proposals.keys().cloned().collect(),
    )
}

/// An office with only its cycle bookkeeping and watermark.
fn cycle_office(office: &FiscalOffice) -> FiscalOffice {
    let mut copy = FiscalOffice::new(office.institution.clone());
    copy.revision = office.revision;
    copy.cycles = office.cycles.clone();
    copy.watermark.clone_from(&office.watermark);
    copy.scope_count = office.scope_count;
    copy
}

/// Merges new report work into what an observer is owed already. The entry
/// keeps the earlier due time, or, for an `Interval` observer's entry
/// (`keep_due`), its own: an interval report is never published on change.
pub(crate) fn merge_queue_entry(
    existing: &mut FiscalReportQueueEntry,
    entry: FiscalReportQueueEntry,
    keep_due: bool,
) {
    if !keep_due {
        existing.due_at = existing.due_at.min(entry.due_at);
    }
    existing.republish |= entry.republish;
    if existing.whole || entry.whole {
        existing.whole = true;
        existing.cycles.clear();
        return;
    }
    for (cycle, parts) in entry.cycles {
        match existing.cycles.get_mut(&cycle) {
            // An empty set already owes every part.
            Some(owed) if owed.is_empty() => {}
            Some(_) if parts.is_empty() => {
                existing.cycles.insert(cycle, BTreeSet::new());
            }
            Some(owed) => owed.extend(parts),
            None => {
                existing.cycles.insert(cycle, parts);
            }
        }
    }
    bound_queue_entry(existing);
}

/// Keeps queued report work within its bounds: a cycle with more parts than
/// [`FISCAL_REPORT_QUEUE_PARTS`] is owed whole, and an entry with more cycles
/// than [`FISCAL_REPORT_QUEUE_CYCLES`] owes the whole report. Owing more is
/// safe: a part whose rows did not change since it was published is skipped.
/// Work the report system queues to finish a publication it must repeat
/// (`republish`) is kept part by part within the report's slots; past
/// [`MAX_FISCAL_REPORT_CYCLE_SLOTS`] cycles, which work merged into it while
/// its holder could not receive it can reach, the whole report is owed and
/// published again.
pub(crate) fn bound_queue_entry(entry: &mut FiscalReportQueueEntry) {
    if entry.republish {
        if entry.cycles.len() > MAX_FISCAL_REPORT_CYCLE_SLOTS {
            entry.cycles.clear();
            entry.whole = true;
        }
        return;
    }
    for parts in entry.cycles.values_mut() {
        if parts.len() > FISCAL_REPORT_QUEUE_PARTS {
            parts.clear();
        }
    }
    if entry.cycles.len() > FISCAL_REPORT_QUEUE_CYCLES {
        entry.cycles.clear();
        entry.whole = true;
    }
}
