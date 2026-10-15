//! A fiscal report record supersedes the record its report line currently
//! holds, and the report system finds that record in the committed knowledge
//! ledger, not in retained boundary evidence.
//!
//! This run publishes a report, seals every boundary so far, and changes the
//! fiscal state again. With the publishing boundaries gone from retained
//! evidence, the sealed run still supersedes exactly the records it published
//! before the seal, as an unsealed fork, a restore from the sealed segments,
//! and its own exact replay do.

mod support;

use canwu_api::{BoundaryRequest, Canwu, CompactedCanwu, KnowledgeHolderRef, KnowledgeRecord};
use canwu_fiscal::{FiscalReportRecord, compute_projection};
use support::{
    Fixture, MemoryEvidenceArchive, OBSERVER, PRINCIPAL, TREASURY_AUTHORITY, assessment,
    fiscal_report, fiscal_state, published_report_records, scenario, sealed_fiscal_state, submit,
    submit_sealed, treasury_scope,
};

/// An assessment of one cycle in the treasury's `scope`-th scope; the
/// treasury's scopes are rows of one report part.
fn assess(action_id: &str, scope: usize) -> support::Draft {
    assessment(
        action_id,
        TREASURY_AUTHORITY,
        &treasury_scope(scope),
        "cycle.1",
    )
}

/// The observer's ledger, oldest record first.
fn ledger(knowledge: &canwu_api::KnowledgeSnapshot) -> Vec<KnowledgeRecord> {
    knowledge
        .for_holder(&KnowledgeHolderRef::Person(PRINCIPAL))
        .expect("the observer's ledger")
        .values()
        .cloned()
        .collect()
}

fn is_summary(record: &KnowledgeRecord) -> bool {
    matches!(
        serde_json::from_value(record.payload.clone()).expect("a fiscal report record"),
        FiscalReportRecord::Summary(_)
    )
}

#[test]
fn a_sealed_run_supersedes_the_report_it_published_before_the_seal() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let mut base = fixture.runtime(scenario(|_| {}));

    // The first fiscal change publishes the observer's report: its summary
    // and its one part.
    submit(&mut base, vec![(PRINCIPAL, assess("before-seal", 0))]);
    let before_seal = published_report_records(&base, PRINCIPAL);
    assert_eq!(before_seal.len(), 2);
    assert!(
        before_seal
            .iter()
            .all(|(record, _)| record.supersedes.is_empty())
    );

    // A quiet boundary admits what the last boundary emitted, and then the
    // whole history seals: no boundary that published a report stays in
    // retained evidence.
    base.settle_boundary(BoundaryRequest::at(base.time()))
        .expect("a quiet boundary before sealing");
    let mut unsealed = base.fork();
    let publishing_boundaries = base.boundaries().to_vec();
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    assert!(
        publishing_boundaries
            .iter()
            .all(|boundary| segment.boundaries.contains(boundary)),
        "the seal covers every publishing boundary"
    );
    let tail = sealed
        .journal_segment_since(segment.end)
        .expect("retained tail");
    assert_eq!(tail.start, tail.end, "nothing is retained after the seal");
    let mut from_sealed = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the compact run");
    assert_eq!(from_sealed.checkpoint_hash(), sealed.checkpoint_hash());

    // The fiscal state changes again in all three runs.
    let after_seal = vec![(PRINCIPAL, assess("after-seal", 1))];
    submit(&mut unsealed, after_seal.clone());
    submit_sealed(&mut sealed, after_seal.clone());
    submit_sealed(&mut from_sealed, after_seal);

    // They agree on the observer's ledger and on the checkpoint.
    let records = ledger(sealed.knowledge());
    assert_eq!(records, ledger(unsealed.knowledge()));
    assert_eq!(records, ledger(from_sealed.knowledge()));
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(from_sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    // Each record published after the seal supersedes exactly the record of
    // its line from before the seal: summary by summary, part by part.
    assert_eq!(records.len(), 4);
    assert_eq!(records[0], before_seal[0].0);
    assert_eq!(records[1], before_seal[1].0);
    for (new, old) in [(&records[2], &records[0]), (&records[3], &records[1])] {
        assert_eq!(is_summary(new), is_summary(old));
        assert_eq!(new.supersedes, vec![old.id]);
    }

    // The observer holds one complete report of both assessments.
    let report = fiscal_report(&unsealed, PRINCIPAL);
    assert!(report.is_complete());
    assert_eq!(report.projection.facts.len(), 2);
    assert_eq!(
        report.projection,
        compute_projection(&fiscal_state(&unsealed), OBSERVER, report.projection.as_of)
            .expect("projection")
            .expect("the observer's projection")
    );

    // Exact replay of the sealed run regenerates every boundary, the one
    // settled after the seal included, and equals the unsealed fork.
    let journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays exactly");
    assert_eq!(
        replayed.snapshot(),
        sealed
            .snapshot_with_segments(vec![segment])
            .expect("the sealed run reconstructs a full snapshot")
    );
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}

/// A fiscal run seals before its first boundary, with its scenario's records
/// and nothing settled (there is no settled evidence yet, so the seal has
/// nothing to take), and after each boundary, and settles exactly as the
/// run that never sealed.
#[test]
fn a_fiscal_run_seals_before_its_first_boundary() {
    use canwu_api::ArchiveStore;
    let fixture = Fixture::new();
    let canwu = fixture.runtime(scenario(|_| {}));
    let mut unsealed = canwu.fork();
    let mut sealed = canwu.into_compacted().expect("compact interface");
    let archive = MemoryEvidenceArchive::default();
    let seal = |sealed: &mut CompactedCanwu| {
        let prepared = sealed.prepare_evidence_seal().expect("a seal prepares");
        let sealed_any = prepared.is_some();
        if let Some(prepared) = prepared {
            archive
                .store_evidence_segment(&prepared.segment)
                .expect("the segment is stored");
            sealed
                .commit_evidence_seal(&prepared.token, &archive)
                .expect("the seal commits");
        }
        sealed_any
    };
    assert!(
        !seal(&mut sealed),
        "nothing is settled before the first boundary"
    );
    for (action, scope) in [("first", 0), ("second", 1)] {
        submit_sealed(&mut sealed, vec![(PRINCIPAL, assess(action, scope))]);
        submit(&mut unsealed, vec![(PRINCIPAL, assess(action, scope))]);
        assert!(seal(&mut sealed), "the settled boundaries seal");
        assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
        assert_eq!(sealed_fiscal_state(&sealed), fiscal_state(&unsealed));
    }
    assert_eq!(sealed_fiscal_state(&sealed).assessments().count(), 2);
}
