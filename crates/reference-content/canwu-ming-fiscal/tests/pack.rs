use canwu_fiscal::{
    FiscalContentSelection, FiscalCoverageStatus, MAX_FISCAL_CATALOG_PERIODS,
    compile_fiscal_content,
};
use canwu_ming_fiscal::{compile_ming_fiscal, ming_fiscal_fixture, ming_fiscal_pack};

#[test]
fn embedded_pack_compiles_across_its_full_longitudinal_scope() {
    let pack = ming_fiscal_pack().expect("embedded pack");
    assert_eq!(pack.manifest.historical_scope.start, 1368);
    assert_eq!(pack.manifest.historical_scope.end, 1683);

    for year in [1368, 1391, 1436, 1581, 1618, 1644, 1662, 1683] {
        let catalog = compile_ming_fiscal(FiscalContentSelection {
            historical_year: year,
            ..FiscalContentSelection::default()
        })
        .expect("full pack should compile at every boundary year");
        assert!(!catalog.active_period_ids(year).is_empty());
        assert_eq!(catalog.coverage.len(), 8 * 8 * 11);
        assert!(catalog.coverage.values().all(|cell| matches!(
            cell.status,
            FiscalCoverageStatus::Supported
                | FiscalCoverageStatus::ArchetypeFallback
                | FiscalCoverageStatus::ExplicitUnknown
                | FiscalCoverageStatus::NotApplicable
        )));
    }
}

#[test]
fn authored_pack_capacity_fails_before_unbounded_catalog_work() {
    let mut pack = ming_fiscal_pack().expect("embedded pack");
    let period = pack.periods[0].clone();
    pack.periods.resize(MAX_FISCAL_CATALOG_PERIODS + 1, period);
    let error = compile_fiscal_content(&pack, FiscalContentSelection::default())
        .expect_err("oversized pack must fail closed");
    assert!(error.message.contains("bounded capacity"));
}

#[test]
fn fixtures_pin_periods_and_regional_reform_expectations() {
    for id in ["hongwu-1391", "wanli-1581", "hongguang-1644"] {
        let fixture = ming_fiscal_fixture(id).expect("fixture");
        let catalog = compile_ming_fiscal(FiscalContentSelection {
            historical_year: fixture.historical_year,
            region_ids: fixture.region_ids.clone(),
            ..FiscalContentSelection::default()
        })
        .expect("fixture catalog");
        assert_eq!(
            catalog.active_period_ids(fixture.historical_year),
            fixture.expected_active_period_ids
        );
        for adoption in fixture.adoptions {
            assert!(catalog.rules.contains_key(&adoption.rule_id));
        }
        for transition in fixture.expected_transition_ids {
            assert!(catalog.transitions.contains_key(&transition));
        }
    }
}

#[test]
fn unsupported_year_is_rejected() {
    let error = compile_ming_fiscal(FiscalContentSelection {
        historical_year: 1684,
        ..FiscalContentSelection::default()
    })
    .expect_err("year after the optional continuation");
    assert!(error.message.contains("outside"));
}

#[test]
fn equal_priority_coverage_overlap_fails_closed() {
    let mut pack = ming_fiscal_pack().expect("pack");
    let mut conflict = pack.coverage[0].clone();
    conflict.id = "conflicting_default".to_owned();
    pack.coverage.push(conflict);
    let error = canwu_fiscal::compile_fiscal_content(
        &pack,
        FiscalContentSelection {
            historical_year: 1581,
            ..FiscalContentSelection::default()
        },
    )
    .expect_err("equal-priority overlap must not depend on file order");
    assert!(error.message.contains("equal-priority conflict"));
}

#[test]
fn every_single_region_and_mechanism_selection_compiles() {
    let pack = ming_fiscal_pack().expect("pack");
    for region in &pack.manifest.region_ids {
        for mechanism in &pack.manifest.mechanisms {
            let catalog = canwu_fiscal::compile_fiscal_content(
                &pack,
                FiscalContentSelection {
                    historical_year: 1581,
                    region_ids: [region.clone()].into_iter().collect(),
                    mechanisms: [*mechanism].into_iter().collect(),
                },
            )
            .unwrap_or_else(|error| panic!("selection {region}/{mechanism:?} failed: {error}"));
            assert_eq!(catalog.coverage.len(), 8);
        }
    }
}

#[test]
fn malformed_versions_sources_transitions_and_unknown_cells_fail_closed() {
    let selection = || FiscalContentSelection {
        historical_year: 1581,
        ..FiscalContentSelection::default()
    };

    let mut invalid_version = ming_fiscal_pack().expect("pack");
    invalid_version.manifest.pack_version = "not-a-version".to_owned();
    assert!(
        canwu_fiscal::compile_fiscal_content(&invalid_version, selection())
            .expect_err("invalid SemVer")
            .message
            .contains("SemVer")
    );

    let mut empty_source = ming_fiscal_pack().expect("pack");
    empty_source.provenance[0].citation.clear();
    assert!(
        canwu_fiscal::compile_fiscal_content(&empty_source, selection())
            .expect_err("empty source")
            .message
            .contains("incomplete")
    );

    let mut cyclic = ming_fiscal_pack().expect("pack");
    let first_id = cyclic.transitions[0].id.clone();
    let second_id = cyclic.transitions[1].id.clone();
    cyclic.transitions[0]
        .prerequisite_ids
        .insert(second_id.clone());
    cyclic.transitions[1].prerequisite_ids.insert(first_id);
    assert!(
        canwu_fiscal::compile_fiscal_content(&cyclic, selection())
            .expect_err("cyclic prerequisites")
            .message
            .contains("cycle")
    );

    let mut unknown_with_behavior = ming_fiscal_pack().expect("pack");
    let rule_id = unknown_with_behavior.rules[0].id.clone();
    let unknown = unknown_with_behavior
        .coverage
        .iter_mut()
        .find(|declaration| declaration.id == "default_explicit_unknown")
        .expect("default unknown declaration");
    unknown.definition_ids.insert(rule_id);
    assert!(
        canwu_fiscal::compile_fiscal_content(&unknown_with_behavior, selection())
            .expect_err("unknown cell with behavior")
            .message
            .contains("non-behavioral")
    );
}
