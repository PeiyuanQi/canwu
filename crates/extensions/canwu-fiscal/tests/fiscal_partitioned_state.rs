//! The fiscal records of a run are always exactly the partition of one valid
//! fiscal state.
//!
//! Each boundary decodes and writes only the records its packets touch, and
//! keeps the invariants across records (office cycle bookkeeping,
//! jurisdiction counts and candidates, the binding directory, the report
//! queue and its index) by construction. These runs drive seeded random
//! sequences of action batches, receipt batches, closes, archive commits,
//! historical-context changes, and a transition, and after every boundary
//! check that the records assemble to a state that validates and that they
//! are exactly what that state partitions into (`audit_fiscal_records`), and
//! that every report the observers hold, one of the treasury and one of the
//! treasury and the granary together, is complete and is what the records
//! derive. At the end, a restore and an exact replay agree with the run.

mod support;

use canwu_api::{Canwu, PersonId};
use canwu_fiscal::{
    FiscalAction, FiscalAdoptionStage, FiscalAuditSeverity, FiscalHistoricalContextPacket,
    FiscalHistoricalMode, FiscalSettledAction, compute_projection,
    fiscal_historical_context_ingress, fiscal_reports,
};
use std::collections::{BTreeMap, BTreeSet};
use support::{
    Draft, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, MemoryArchiveStore,
    PRINCIPAL, REFORM, REFORMED_RULE, TREASURY_AUTHORITY, TREASURY_SCOPES, archive, assessment,
    close, collection, draft, enqueue_batches, enqueue_results, execution_claim, fiscal_state,
    granary, observer_binding, receipt_packet, result_versions, scenario, settle, treasury,
    treasury_scope,
};

/// An observer of both institutions, whose parts mix the scopes of two
/// offices.
const BOTH: &str = "observer.both";

/// Every report the principal holds is complete and is what the records
/// derive, and every observer whose institutions hold a hot book has one.
fn assert_reports_current(canwu: &Canwu) {
    let state = fiscal_state(canwu);
    let viewer = canwu.viewer_for_actor(PRINCIPAL).expect("viewer");
    let reports = fiscal_reports(&viewer).expect("reports");
    for observer in [support::OBSERVER, BOTH] {
        let derived = compute_projection(&state, observer, canwu.time())
            .expect("projection")
            .expect("an observer");
        let Some(report) = reports.get(observer) else {
            assert!(derived.facts.is_empty(), "{observer} holds no report");
            continue;
        };
        assert!(report.is_complete(), "{observer} is complete");
        assert_eq!(
            report.projection.facts, derived.facts,
            "{observer} reports what the records derive"
        );
    }
}

/// A small deterministic generator, so that every run of a seed is the same.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound).expect("bound")).expect("index")
    }
}

/// What the run has made so far, so that later steps name things that exist.
#[derive(Default)]
struct Ledger {
    /// Assessment ID by (scope, cycle).
    assessments: BTreeMap<(String, String), Vec<String>>,
    /// Requests not yet receipted: (scope, request, quantity).
    requests: Vec<(String, String, u64)>,
    /// Books closed and not yet archived.
    closed: BTreeSet<(String, String)>,
    serial: usize,
}

fn scopes() -> Vec<(PersonId, &'static str, String)> {
    (0..TREASURY_SCOPES)
        .map(|index| (PRINCIPAL, TREASURY_AUTHORITY, treasury_scope(index)))
        .chain(std::iter::once((
            GRANARY_PRINCIPAL,
            GRANARY_AUTHORITY,
            GRANARY_SCOPE.to_owned(),
        )))
        .collect()
}

/// One step's commands: a few random actions on random scopes, assessing
/// under `rule`.
fn actions(
    random: &mut Random,
    ledger: &mut Ledger,
    round: usize,
    rule: &str,
) -> Vec<(PersonId, Draft)> {
    let scopes = scopes();
    let mut commands = Vec::new();
    for _ in 0..=random.below(10) {
        ledger.serial += 1;
        let serial = ledger.serial;
        let (issuer, binding, scope) = scopes[random.below(scopes.len())].clone();
        let cycle = format!("{:04}", 2000 + round / 4 + random.below(2));
        match random.below(5) {
            0 | 1 => {
                let mut action = assessment(&format!("assess.{serial}"), binding, &scope, &cycle);
                if let FiscalAction::OpenAssessment { rule_id, .. } = &mut action.action {
                    rule.clone_into(rule_id);
                }
                if !ledger.closed.contains(&(scope.clone(), cycle.clone())) {
                    ledger
                        .assessments
                        .entry((scope, cycle))
                        .or_default()
                        .push(format!("assess.{serial}.assessment"));
                }
                commands.push((issuer, action));
            }
            2 => {
                let open: Vec<_> = ledger
                    .assessments
                    .iter()
                    .filter(|(key, _)| key.0 == scope && !ledger.closed.contains(*key))
                    .flat_map(|(_, ids)| ids.clone())
                    .collect();
                if let Some(target) = open.get(random.below(open.len().max(1))) {
                    let request = format!("request.{serial}");
                    let quantity = 1 + u64::try_from(random.below(20)).expect("quantity");
                    let mut draft = collection(
                        &format!("authorize.{serial}"),
                        &scope,
                        target,
                        &request,
                        quantity,
                    );
                    binding.clone_into(&mut draft.binding);
                    if let FiscalAction::AuthorizeExecution { target: payee, .. } =
                        &mut draft.action
                        && binding == GRANARY_AUTHORITY
                    {
                        *payee = support::granary();
                    }
                    ledger.requests.push((scope.clone(), request, quantity));
                    commands.push((issuer, draft));
                }
            }
            3 => {
                let open: Vec<_> = ledger
                    .assessments
                    .keys()
                    .filter(|key| key.0 == scope && !ledger.closed.contains(*key))
                    .cloned()
                    .collect();
                if let Some((scope, cycle)) = open.get(random.below(open.len().max(1))).cloned() {
                    let mut draft = close(&format!("close.{serial}"), &scope, &cycle);
                    binding.clone_into(&mut draft.binding);
                    ledger.closed.insert((scope.clone(), cycle.clone()));
                    ledger.requests.retain(|(request_scope, request, _)| {
                        request_scope != &scope || !request.is_empty()
                    });
                    commands.push((issuer, draft));
                }
            }
            _ => {
                let targets: Vec<_> = ledger
                    .assessments
                    .iter()
                    .filter(|(key, _)| key.0 == scope)
                    .flat_map(|(_, ids)| ids.clone())
                    .collect();
                if let Some(target) = targets.get(random.below(targets.len().max(1))) {
                    commands.push((
                        issuer,
                        draft(
                            &format!("audit.{serial}"),
                            binding,
                            FiscalAction::RecordAudit {
                                audit_id: format!("finding.{serial}"),
                                scope_binding_id: scope.clone(),
                                target_id: target.clone(),
                                severity: FiscalAuditSeverity::Notice,
                                finding: "checked".to_owned(),
                                evidence: BTreeSet::new(),
                            },
                        ),
                    ));
                }
            }
        }
    }
    commands
}

/// Collects some open requests in full, recording their results in one
/// boundary and their receipts, as one batch, in the next.
fn receipts(canwu: &mut Canwu, random: &mut Random, ledger: &mut Ledger) {
    if ledger.requests.is_empty() {
        return;
    }
    let count = 1 + random.below(ledger.requests.len());
    let taken: Vec<_> = ledger.requests.drain(..count).collect();
    let results: Vec<_> = taken
        .iter()
        .map(|(scope, request, quantity)| {
            let mut claim =
                execution_claim(&format!("result.{request}"), scope, request, *quantity);
            if scope == GRANARY_SCOPE {
                claim.target = support::granary();
            }
            (false, claim)
        })
        .collect();
    enqueue_results(canwu, &results);
    settle(canwu);
    let _ = fiscal_state(canwu);
    let now = canwu.time();
    let packets: Vec<_> = results
        .iter()
        .zip(result_versions(canwu, &results))
        .map(|((_, claim), version)| {
            receipt_packet(
                &format!("receipt.{}", claim.request_id),
                &claim.scope_binding_id,
                &claim.request_id,
                [version],
            )
        })
        .collect();
    canwu_fiscal::enqueue_execution_receipts(canwu, now, &packets).expect("receipts");
}

fn context(canwu: &mut Canwu, year: i32) {
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(
            fiscal_historical_context_ingress(
                now,
                &FiscalHistoricalContextPacket {
                    year,
                    mode: FiscalHistoricalMode::Counterfactual,
                },
            )
            .expect("context packet"),
        )
        .expect("context ingress");
}

/// Drops what the archive took from what the run tracks.
fn forget_archived(canwu: &Canwu, ledger: &mut Ledger) {
    let state = fiscal_state(canwu);
    let hot = |(scope, cycle): &(String, String)| {
        state
            .ledger(scope)
            .is_some_and(|ledger| ledger.books.contains_key(cycle))
    };
    ledger.closed.retain(hot);
    ledger.assessments.retain(|key, _| hot(key));
}

/// Closes every open book, archives every closed one, moves the context into
/// the reform's window, and applies the reform.
fn reform(canwu: &mut Canwu, store: &MemoryArchiveStore, ledger: &mut Ledger, round: usize) {
    let state = fiscal_state(canwu);
    let closes: Vec<_> = state
        .books()
        .filter(|(_, book)| book.is_open())
        .map(|(scope_ledger, book)| {
            let (issuer, binding) = if scope_ledger.scope.id == GRANARY_SCOPE {
                (GRANARY_PRINCIPAL, GRANARY_AUTHORITY)
            } else {
                (PRINCIPAL, TREASURY_AUTHORITY)
            };
            let mut draft = close(
                &format!(
                    "final.close.{}.{}",
                    scope_ledger.scope.id, book.accounting_cycle_id
                ),
                &scope_ledger.scope.id,
                &book.accounting_cycle_id,
            );
            binding.clone_into(&mut draft.binding);
            (issuer, draft)
        })
        .collect();
    enqueue_batches(canwu, closes);
    settle(canwu);
    while fiscal_state(canwu).books().next().is_some() {
        archive(canwu, store, 64);
    }
    forget_archived(canwu, ledger);
    ledger.requests.clear();
    context(canwu, 165);
    settle(canwu);
    let state = fiscal_state(canwu);
    let region = state.jurisdiction("region.a").expect("the region");
    assert!(
        region
            .transition_candidates
            .contains_key(&format!("{REFORM}::region.a"))
    );
    enqueue_batches(
        canwu,
        vec![(
            PRINCIPAL,
            draft(
                &format!("reform.{round}"),
                TREASURY_AUTHORITY,
                FiscalAction::ApplyTransition {
                    transition_id: REFORM.to_owned(),
                    target_scope_bindings: BTreeMap::from([(
                        REFORMED_RULE.to_owned(),
                        treasury_scope(0),
                    )]),
                },
            ),
        )],
    );
    settle(canwu);
    let state = fiscal_state(canwu);
    assert!(
        state
            .adoptions()
            .filter(|adoption| adoption.rule_id == support::RULE)
            .all(|adoption| adoption.stage == FiscalAdoptionStage::Suspended)
    );
    // The region's counts and candidates follow its ledgers.
    let region = state.jurisdiction("region.a").expect("the region");
    assert_eq!(region.adoption_counts[REFORMED_RULE].operational, 1);
    assert!(region.transition_candidates.is_empty());
}

fn run(seed: u64, rounds: usize) {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        state.add_observer(observer_binding(
            BOTH,
            PRINCIPAL,
            BTreeSet::from([treasury(), granary()]),
        ));
    }));
    let store = MemoryArchiveStore::default();
    let mut random = Random(seed);
    let mut ledger = Ledger::default();
    // Random rounds under the old rule, the context moving in and out of
    // the reform's window; then every book closed and archived, the reform
    // applied, which suspends the old rule in every scope of the region and
    // implements the new one in one; then random rounds under the new rule.
    for round in 0..rounds {
        let rule = if round < rounds / 2 {
            support::RULE
        } else {
            REFORMED_RULE
        };
        if round == rounds / 2 {
            reform(&mut canwu, &store, &mut ledger, round);
        }
        enqueue_batches(&mut canwu, actions(&mut random, &mut ledger, round, rule));
        match random.below(4) {
            0 => receipts(&mut canwu, &mut random, &mut ledger),
            1 if round < rounds / 2 => {
                context(&mut canwu, if random.below(2) == 0 { 150 } else { 165 });
            }
            _ => {}
        }
        settle(&mut canwu);
        assert_reports_current(&canwu);
        let state = fiscal_state(&canwu);
        if round % 5 == 4 && state.books().any(|(_, book)| !book.is_open()) {
            archive(&mut canwu, &store, 1 + random.below(8));
            forget_archived(&canwu, &mut ledger);
            assert_reports_current(&canwu);
        }
    }
    let settled = FiscalSettledAction::from_events(canwu.events());
    assert!(settled.len() > rounds, "{} outcomes", settled.len());
    let receipts = canwu
        .events()
        .iter()
        .filter(|event| {
            event
                .summary
                .starts_with("Recorded external fiscal execution evidence")
        })
        .count();
    assert!(receipts > 0, "the run settled receipts");
    fixture.assert_restore_and_replay(&canwu);
}

#[test]
fn random_runs_keep_every_record_the_partition_of_a_valid_state() {
    for seed in [0x5EED_0001, 0x5EED_0002, 0x5EED_0003] {
        run(seed, 60);
    }
}
