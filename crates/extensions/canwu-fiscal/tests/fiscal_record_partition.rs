//! A run's fiscal records are exactly the partition of one valid fiscal
//! state, and a run is refused when it is created with records that are not.
//!
//! The partition names every record by what owns it: the root and the archive
//! head by `primary`, an office by the key of its institution, a ledger by its
//! scope binding, a jurisdiction by its region, and an observer by its
//! binding; the binding directory, scope index, and report queue it derives
//! by the digest of what they hold. Only the root's catalog and an observer's
//! holder are referenced. Activation assembles the state, validates it, and
//! checks every record against the partition of that state, so a record
//! under another owner's identity, a derived field out of step with what it
//! derives from, and a directory or index that does not match the state are
//! each refused. An empty directory bucket left over from an earlier
//! partition is not.

mod support;

use canwu_api::{DomainRecord, DomainReferenceTarget, EntityRef, SimDuration};
use canwu_fiscal::{
    FiscalDirectoryBucket, FiscalIndexPage, FiscalJurisdiction, FiscalObserver,
    FiscalObserverBinding, FiscalOffice, FiscalReportCadence, FiscalScopeLedger, FiscalState,
    audit_fiscal_record_set, fiscal_office_key,
};
use std::collections::{BTreeMap, BTreeSet};
use support::{
    Fixture, GRANARY_SCOPE, OBSERVER, PRINCIPAL, RULE, TREASURY_AUTHORITY, catalog, granary,
    observer_binding, scenario, treasury, treasury_scope,
};

/// An observer binding on a monthly interval.
const INTERVAL: &str = "observer.interval";

fn fiscal(records: &[DomainRecord]) -> Vec<&DomainRecord> {
    records
        .iter()
        .filter(|record| record.reference.kind.namespace == "canwu.fiscal")
        .collect()
}

fn of_kind<'a>(records: &'a [DomainRecord], kind: &str) -> Vec<&'a DomainRecord> {
    fiscal(records)
        .into_iter()
        .filter(|record| record.reference.kind.name == kind)
        .collect()
}

fn is_hex(value: &str, digits: usize) -> bool {
    value.len() == digits
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[test]
#[allow(clippy::too_many_lines)]
fn the_partition_names_every_record_by_what_owns_it() {
    let records = scenario(|_| {}).domain_records;
    let kinds: BTreeMap<&str, usize> =
        fiscal(&records)
            .into_iter()
            .fold(BTreeMap::new(), |mut kinds, record| {
                *kinds
                    .entry(record.reference.kind.name.as_str())
                    .or_default() += 1;
                kinds
            });
    // One root, one archive head, two offices, six ledgers (four treasury
    // scopes, the granary's, and the pending scope), one region, one
    // observer, the catalog, and the derived records.
    assert_eq!(kinds["catalog"], 1);
    assert_eq!(kinds["root"], 1);
    assert_eq!(kinds["archive"], 1);
    assert_eq!(kinds["office"], 2);
    assert_eq!(kinds["ledger"], 6);
    assert_eq!(kinds["jurisdiction"], 1);
    assert_eq!(kinds["observer"], 1);
    assert_eq!(kinds["report-queue"], 1);
    assert_eq!(kinds["report-schedule"], 1);
    assert!(kinds["directory"] >= 1);
    // An index page of each office and of the region.
    assert_eq!(kinds["index"], 3);

    for record in of_kind(&records, "root")
        .into_iter()
        .chain(of_kind(&records, "archive"))
        .chain(of_kind(&records, "report-schedule"))
    {
        assert_eq!(record.reference.id, "primary");
    }
    let root = &of_kind(&records, "root")[0];
    assert_eq!(root.references.len(), 1);
    assert_eq!(root.references[0].role, "catalog");
    for record in of_kind(&records, "office") {
        let office: FiscalOffice = serde_json::from_value(record.payload.clone()).expect("office");
        assert_eq!(record.reference.id, fiscal_office_key(&office.institution));
        let key = record
            .reference
            .id
            .strip_prefix("office.")
            .expect("an office key");
        assert!(is_hex(key, 32), "{}", record.reference.id);
        assert!(record.references.is_empty());
    }
    let mut ranks = BTreeMap::<String, BTreeSet<u32>>::new();
    for record in of_kind(&records, "ledger") {
        let ledger: FiscalScopeLedger =
            serde_json::from_value(record.payload.clone()).expect("ledger");
        assert_eq!(record.reference.id, ledger.scope.id);
        assert_eq!(ledger.office, fiscal_office_key(&ledger.scope.institution));
        assert!(ranks.entry(ledger.office).or_default().insert(ledger.rank));
        assert!(record.references.is_empty());
    }
    // An office's scopes are ranked 0, 1, ... in ID order.
    assert_eq!(
        ranks[&fiscal_office_key(&treasury())],
        (0..5).collect::<BTreeSet<_>>()
    );
    assert_eq!(ranks[&fiscal_office_key(&granary())], BTreeSet::from([0]));
    for record in of_kind(&records, "jurisdiction") {
        let jurisdiction: FiscalJurisdiction =
            serde_json::from_value(record.payload.clone()).expect("jurisdiction");
        assert_eq!(record.reference.id, jurisdiction.region_id);
        assert_eq!(jurisdiction.scope_count, 6);
        assert_eq!(jurisdiction.adoption_counts[RULE].live, 5);
    }
    let observer = &of_kind(&records, "observer")[0];
    let binding: FiscalObserver =
        serde_json::from_value(observer.payload.clone()).expect("observer");
    assert_eq!(observer.reference.id, binding.binding.id);
    assert_eq!(observer.reference.id, OBSERVER);
    assert_eq!(observer.references.len(), 1);
    assert_eq!(observer.references[0].role, "holder");
    assert_eq!(
        observer.references[0].target,
        DomainReferenceTarget::Core(EntityRef::Person(PRINCIPAL))
    );
    let mut directory = BTreeMap::new();
    for record in of_kind(&records, "directory") {
        let bucket = record
            .reference
            .id
            .strip_prefix("dir.")
            .expect("a directory bucket");
        assert!(is_hex(bucket, 3), "{}", record.reference.id);
        let entries: FiscalDirectoryBucket =
            serde_json::from_value(record.payload.clone()).expect("bucket");
        assert!(!entries.is_empty());
        directory.extend(entries.bindings);
    }
    assert_eq!(
        directory.get(TREASURY_AUTHORITY),
        Some(&fiscal_office_key(&treasury()))
    );
    for record in of_kind(&records, "report-queue") {
        let page = record
            .reference
            .id
            .strip_prefix("queue.")
            .expect("a queue page");
        assert!(is_hex(page, 3), "{}", record.reference.id);
    }
    let mut indexed = BTreeSet::new();
    for record in of_kind(&records, "index") {
        let (owner, page) = record
            .reference
            .id
            .strip_prefix("index.")
            .and_then(|rest| rest.split_once('.'))
            .expect("an index page");
        assert!(is_hex(owner, 32), "{}", record.reference.id);
        assert_eq!(page, "0");
        let page: FiscalIndexPage = serde_json::from_value(record.payload.clone()).expect("page");
        assert_eq!(page.owner.page_id(page.page), record.reference.id);
        indexed.extend(page.scopes);
    }
    assert!(indexed.contains(GRANARY_SCOPE));
    assert!(indexed.contains(&treasury_scope(3)));

    // The partition is a fixed point: the state its records assemble to
    // partitions into the same records.
    let state = audit_fiscal_record_set(fiscal(&records)).expect("the partition audits");
    let again = state.into_records(&catalog()).expect("partitioned again");
    let original: BTreeMap<_, _> = fiscal(&records)
        .into_iter()
        .filter(|record| record.reference.kind.name != "catalog")
        .map(|record| (record.reference.clone(), record.payload.clone()))
        .collect();
    let repeated: BTreeMap<_, _> = again
        .into_iter()
        .map(|record| (record.reference, record.payload))
        .collect();
    assert_eq!(original, repeated);
}

/// The message a run is refused with when it is created from the base
/// scenario with its records changed by `change`.
fn refusal(change: impl FnOnce(&mut Vec<DomainRecord>)) -> String {
    let fixture = Fixture::new();
    let mut scenario = scenario(|_| {});
    change(&mut scenario.domain_records);
    fixture
        .try_runtime(scenario)
        .err()
        .expect("changed fiscal records must be refused")
        .message
}

fn record<'a>(records: &'a mut [DomainRecord], kind: &str, id: &str) -> &'a mut DomainRecord {
    records
        .iter_mut()
        .find(|record| record.reference.kind.name == kind && record.reference.id == id)
        .unwrap_or_else(|| panic!("{kind} {id}"))
}

fn bucket_of<'a>(records: &'a mut [DomainRecord], binding: &str) -> &'a mut DomainRecord {
    records
        .iter_mut()
        .find(|record| {
            record.reference.kind.name == "directory"
                && record.payload["bindings"].get(binding).is_some()
        })
        .unwrap_or_else(|| panic!("the bucket of {binding}"))
}

#[test]
fn a_record_under_another_owner_is_refused() {
    // An office under a key that is not its institution's.
    let message = refusal(|records| {
        record(records, "office", &fiscal_office_key(&granary()))
            .reference
            .id = format!("office.{}", "0".repeat(32));
    });
    assert!(message.contains("office"), "{message}");

    // A ledger that names another office than its institution's.
    let message = refusal(|records| {
        record(records, "ledger", &treasury_scope(0)).payload["office"] =
            serde_json::json!(fiscal_office_key(&granary()));
    });
    assert!(message.contains("names another office"), "{message}");
}

#[test]
fn a_derived_field_out_of_step_is_refused() {
    // A ledger ranked where another scope of its office is.
    let message = refusal(|records| {
        record(records, "ledger", &treasury_scope(0)).payload["rank"] = serde_json::json!(1);
    });
    assert!(message.contains("derive"), "{message}");

    // A region that counts one adoption too many.
    let message = refusal(|records| {
        let region = record(records, "jurisdiction", "region.a");
        let live = region.payload["adoption_counts"][RULE]["live"]
            .as_u64()
            .expect("a count");
        region.payload["adoption_counts"][RULE]["live"] = serde_json::json!(live + 1);
    });
    assert!(message.contains("derive"), "{message}");
}

#[test]
fn a_directory_or_index_that_does_not_match_the_state_is_refused() {
    // A binding missing from its bucket.
    let message = refusal(|records| {
        let bucket = bucket_of(records, TREASURY_AUTHORITY);
        bucket.payload["bindings"]
            .as_object_mut()
            .expect("bindings")
            .remove(TREASURY_AUTHORITY);
    });
    assert!(
        message.contains("differs from the partition") || message.contains("is missing"),
        "{message}"
    );

    // A binding the state does not hold, added to a bucket.
    let message = refusal(|records| {
        bucket_of(records, TREASURY_AUTHORITY).payload["bindings"]["authority.ghost"] =
            serde_json::json!(fiscal_office_key(&treasury()));
    });
    assert!(message.contains("differs from the partition"), "{message}");

    // A whole bucket removed.
    let message = refusal(|records| {
        let id = bucket_of(records, TREASURY_AUTHORITY).reference.id.clone();
        records.retain(|record| record.reference.id != id);
    });
    assert!(message.contains("is missing"), "{message}");

    // An index page whose scopes are out of rank order.
    let message = refusal(|records| {
        let page = records
            .iter_mut()
            .find(|record| {
                record.reference.kind.name == "index"
                    && record.payload["scopes"]
                        .as_array()
                        .is_some_and(|scopes| scopes.len() > 1)
            })
            .expect("an index page of several scopes");
        page.payload["scopes"]
            .as_array_mut()
            .expect("scopes")
            .reverse();
    });
    assert!(message.contains("differs from the partition"), "{message}");
}

#[test]
fn an_empty_directory_bucket_left_over_is_accepted() {
    let fixture = Fixture::new();
    let mut scenario = scenario(|_| {});
    let mut leftover = scenario
        .domain_records
        .iter()
        .find(|record| record.reference.kind.name == "directory")
        .expect("a bucket")
        .clone();
    leftover.reference.id = (0..4_096)
        .map(|bucket| format!("dir.{bucket:03x}"))
        .find(|id| {
            !scenario
                .domain_records
                .iter()
                .any(|record| &record.reference.id == id)
        })
        .expect("a bucket not in use");
    leftover.payload = serde_json::to_value(FiscalDirectoryBucket::default()).expect("payload");
    scenario.domain_records.push(leftover);
    let canwu = fixture
        .try_runtime(scenario)
        .expect("a leftover empty bucket");
    let state: FiscalState = support::fiscal_state(&canwu);
    assert!(state.authority_binding(TREASURY_AUTHORITY).is_some());
}

/// The chain from the report-queue pages to the root is derived and
/// checked: the report schedule keeps each page's earliest due time and the
/// root the least of them, an active interval binding has its one entry at
/// one of its due times, and the rotation's cursor exists only while an
/// entry is parked. A run whose records break the chain is refused.
#[test]
fn a_report_schedule_out_of_step_is_refused() {
    let interval_refusal = |change: fn(&mut Vec<DomainRecord>)| {
        let fixture = Fixture::new();
        let mut scenario = scenario(|state| {
            state.add_observer(FiscalObserverBinding {
                cadence: FiscalReportCadence::Interval {
                    every: SimDuration::days(30),
                    offset: SimDuration::days(3),
                },
                ..observer_binding(INTERVAL, PRINCIPAL, BTreeSet::from([treasury()]))
            });
        });
        assert!(fixture.try_runtime(scenario.clone()).is_ok());
        change(&mut scenario.domain_records);
        fixture
            .try_runtime(scenario)
            .err()
            .expect("changed fiscal records must be refused")
            .message
    };
    // The root's earliest due time is not the schedule's least.
    let message = interval_refusal(|records| {
        record(records, "root", "primary").payload["earliest_due"] = serde_json::json!(1);
    });
    assert!(message.contains("derive"), "{message}");
    // A page's earliest due time is not its entries'.
    let message = interval_refusal(|records| {
        let schedule = record(records, "report-schedule", "primary");
        for due in schedule.payload["page_due"]
            .as_object_mut()
            .expect("page due times")
            .values_mut()
        {
            *due = serde_json::json!(due.as_i64().expect("a time") + 1);
        }
    });
    assert!(message.contains("derive"), "{message}");
    // A cursor while nothing is parked.
    let message = interval_refusal(|records| {
        record(records, "root", "primary").payload["parked_cursor"] = serde_json::json!(OBSERVER);
    });
    assert!(message.contains("cursor"), "{message}");
    // The interval binding's entry is not at one of its due times.
    let message = interval_refusal(|records| {
        let page = records
            .iter_mut()
            .find(|record| {
                record.reference.kind.name == "report-queue"
                    && record.payload["entries"].get(INTERVAL).is_some()
            })
            .expect("the interval binding's page");
        page.payload["entries"][INTERVAL]["due_at"] = serde_json::json!(1);
    });
    assert!(message.contains("due times"), "{message}");
}
