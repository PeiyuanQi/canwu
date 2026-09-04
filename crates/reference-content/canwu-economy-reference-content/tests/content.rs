use canwu_economy_reference_content::{
    AuthoredValueOrigin, CalibrationStatus, CoverageDeclarationId, CoverageStatus,
    ModelClassification, ResourceCapabilityStage, RuleRevisionId, china_industrialization_fixture,
    compile_content_pack, fixture_ids, ming_workshop_fixture, synthetic_grain_fixture,
};

#[test]
fn all_reference_fixtures_compile_with_exhaustive_required_coverage() {
    let fixtures = [
        synthetic_grain_fixture(),
        ming_workshop_fixture(),
        china_industrialization_fixture(),
    ];
    assert_eq!(fixture_ids().len(), fixtures.len());
    for pack in fixtures {
        let expected = pack.manifest.required_coverage_keys.len();
        let compiled = compile_content_pack(&pack).expect("fixture must compile");
        assert_eq!(compiled.coverage.len(), expected);
        assert!(compiled.coverage.values().all(|cell| matches!(
            cell.status,
            CoverageStatus::Supported
                | CoverageStatus::ArchetypeFallback
                | CoverageStatus::ExplicitUnknown
                | CoverageStatus::NotApplicable
        )));
    }
}

#[test]
fn equal_priority_overlap_and_missing_model_card_fail_closed() {
    let mut conflict = ming_workshop_fixture();
    let mut duplicate = conflict.coverage[0].clone();
    duplicate.id =
        CoverageDeclarationId::new("canwu.economy:coverage-declaration:ming-workshop-conflict.v1")
            .expect("test ID");
    conflict.coverage.push(duplicate);
    let error = compile_content_pack(&conflict).expect_err("equal priority must fail");
    assert!(error.message.contains("equal-priority conflict"));

    let mut missing_card = ming_workshop_fixture();
    missing_card.model_cards.clear();
    let error = compile_content_pack(&missing_card).expect_err("missing model card must fail");
    assert!(error.message.contains("unavailable"));
}

#[test]
fn unknown_and_not_applicable_cells_cannot_authorize_behavior() {
    let mut pack = synthetic_grain_fixture();
    let definition = pack.definitions[0].id.clone();
    let model_card = pack.model_cards[0].id.clone();
    let unknown = pack
        .coverage
        .iter_mut()
        .find(|coverage| coverage.status == CoverageStatus::ExplicitUnknown)
        .expect("explicit-unknown price cell");
    unknown.definition_ids.insert(definition);
    unknown.model_card_ids.insert(model_card);
    let error = compile_content_pack(&pack).expect_err("unknown behavior must fail closed");
    assert!(error.message.contains("non-behavioral"));

    let mut pack = synthetic_grain_fixture();
    let unknown = pack
        .coverage
        .iter_mut()
        .find(|coverage| coverage.status == CoverageStatus::ExplicitUnknown)
        .expect("explicit-unknown price cell");
    unknown.status = CoverageStatus::NotApplicable;
    let compiled = compile_content_pack(&pack).expect("empty not-applicable cell is valid");
    let key = pack
        .manifest
        .required_coverage_keys
        .iter()
        .find(|key| {
            key.mechanism == canwu_economy_reference_content::EconomyMechanism::PricePressure
        })
        .expect("price key");
    assert!(compiled.behavior_for(key).is_err());
}

#[test]
fn lower_priority_archetype_cannot_override_local_evidence() {
    let mut pack = ming_workshop_fixture();
    let local = pack.coverage[0].clone();
    let mut broad = local.clone();
    broad.id = CoverageDeclarationId::new(
        "canwu.economy:coverage-declaration:broad-workshop-archetype.v1",
    )
    .expect("test ID");
    broad.priority = local.priority - 1;
    broad.selector.regions.clear();
    pack.coverage.push(broad);
    let compiled = compile_content_pack(&pack).expect("priority resolves explicitly");
    assert!(
        compiled
            .coverage
            .values()
            .all(|cell| cell.declaration_id == local.id)
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn historical_fixture_geography_periods_and_locators_are_exact() {
    let ming = ming_workshop_fixture();
    let ming_key = ming
        .manifest
        .required_coverage_keys
        .iter()
        .next()
        .expect("Ming coverage key");
    assert_eq!(
        ming_key.region.as_str(),
        "canwu.economy:region:songjiang-lower-yangzi"
    );
    assert_eq!(
        ming_key
            .historical_years
            .as_ref()
            .map(|value| (value.start_year, value.end_year_exclusive,)),
        Some((1450, 1645))
    );
    let ming_citations: Vec<_> = ming
        .model_cards
        .iter()
        .flat_map(|card| &card.citations)
        .collect();
    assert!(ming_citations.iter().any(|citation| {
        citation.url
            == "https://www.lse.ac.uk/Economic-History/Assets/Documents/Research/GEHN/Padua/PADUAZurndorfer.pdf"
            && citation
                .citation
                .contains("The Pre-modern History of Cotton in China")
    }));
    assert!(ming_citations.iter().any(|citation| {
        citation.url == "https://doi.org/10.1163/156852011X614028"
            && citation.citation.contains("Great Divergence")
    }));

    let hanyeping = china_industrialization_fixture();
    assert!(hanyeping.manifest.required_coverage_keys.iter().all(|key| {
        key.region.as_str() == "canwu.economy:region:middle-yangzi-hubei-jiangxi-industrial-chain"
            && key.historical_years.as_ref().is_some_and(|years| {
                (years.start_year == 1896 && years.end_year_exclusive == 1897)
                    || (years.start_year == 1904 && years.end_year_exclusive == 1905)
                    || (years.start_year == 1906 && years.end_year_exclusive == 1908)
                    || (years.start_year == 1896 && years.end_year_exclusive == 1908)
                    || (years.start_year == 1908 && years.end_year_exclusive == 1912)
            })
    }));
    let citations: Vec<_> = hanyeping
        .model_cards
        .iter()
        .flat_map(|card| &card.citations)
        .collect();
    assert!(citations.iter().any(|citation| {
        citation.citation.starts_with("Yun Liu,")
            && citation.url == "https://doi.org/10.1080/00076790903469612"
            && citation.locator.contains("pp. 62-64")
    }));
    assert!(citations.iter().any(|citation| {
        citation.citation.contains("Don't Tread On Me")
            && citation.url == "https://ihss.hku.hk/wp-content/uploads/2020/09/cbh-15-1.pdf"
            && citation.locator.contains("PDF p. 1")
    }));
    assert!(citations.iter().any(|citation| {
        citation.url == "https://doi.org/10.1177/009770040102700202"
            && citation.citation.contains("The Case of the Wen Lineage")
            && citation.citation.contains("202-228")
    }));
    assert!(citations.iter().any(|citation| {
        citation.url == "https://doi.org/10.1515/9780804794732"
            && citation.locator.contains("pp. 117-118")
            && !citation.locator.contains("completed")
    }));
    assert!(citations.iter().any(|citation| {
        citation.url == "https://doi.org/10.1525/9780520954038"
            && citation.locator.contains("pp. 17-20")
    }));
    assert!(citations.iter().any(|citation| {
        citation.url == "https://m.voc.com.cn/xhn/news/202007/14436245.html"
            && citation.locator.contains("Yangsanshi, Liling, in 1903")
            && citation.locator.contains("extended to Zhuzhou in 1905")
            && citation.locator.contains("rail to riverboat")
            && citation.citation.contains("Zhu Li")
            && citation.citation.contains("Hunan Daily New Media")
    }));
    let early_capability = hanyeping
        .definitions
        .iter()
        .filter_map(|definition| definition.resource_capability.as_ref())
        .find(|capability| {
            capability
                .coverage_key
                .historical_years
                .as_ref()
                .is_some_and(|years| years.start_year == 1896 && years.end_year_exclusive == 1897)
        })
        .expect("early precursor capability");
    assert_eq!(
        early_capability.stage,
        ResourceCapabilityStage::ObservedSurveyed
    );
    assert!(early_capability.route_access_evidence_class.is_none());
    let operating_capability = hanyeping
        .definitions
        .iter()
        .filter_map(|definition| definition.resource_capability.as_ref())
        .find(|capability| {
            capability
                .coverage_key
                .historical_years
                .as_ref()
                .is_some_and(|years| years.start_year == 1904 && years.end_year_exclusive == 1905)
        })
        .expect("operating precursor capability");
    assert_eq!(
        operating_capability.stage,
        ResourceCapabilityStage::OperatingSite
    );
    assert!(operating_capability.route_access_evidence_class.is_none());
    let route_capability = hanyeping
        .definitions
        .iter()
        .filter_map(|definition| definition.resource_capability.as_ref())
        .find(|capability| {
            capability
                .coverage_key
                .historical_years
                .as_ref()
                .is_some_and(|years| years.start_year == 1906 && years.end_year_exclusive == 1908)
        })
        .expect("whole-year route capability");
    assert_eq!(
        route_capability.stage,
        ResourceCapabilityStage::RouteAccessible
    );
    assert!(route_capability.route_access_evidence_class.is_some());
    assert!(
        hanyeping
            .definitions
            .iter()
            .filter_map(|definition| definition.resource_capability.as_ref())
            .filter(|capability| capability.stage == ResourceCapabilityStage::RouteAccessible)
            .all(|capability| capability
                .coverage_key
                .historical_years
                .as_ref()
                .is_some_and(|years| years.start_year >= 1906))
    );
}

#[test]
fn historical_fact_cards_are_separate_from_gameplay_values_and_rules() {
    for pack in [ming_workshop_fixture(), china_industrialization_fixture()] {
        let compiled = compile_content_pack(&pack).expect("historical fixture");
        for definition in compiled.definitions.values() {
            for field in &definition.numeric_fields {
                let card = &compiled.model_cards[&field.model_card_id];
                assert_eq!(field.origin, AuthoredValueOrigin::GameplayCalibration);
                assert_eq!(card.classification, ModelClassification::Synthetic);
                assert_eq!(card.calibration_status, CalibrationStatus::Uncalibrated);
            }
            for rule in &definition.causal_rules {
                let card = &compiled.model_cards[&rule.model_card_id];
                assert_eq!(card.classification, ModelClassification::Synthetic);
                assert!(card.rule_revisions.contains(&rule.rule_revision));
            }
        }
        assert!(compiled.model_cards.values().any(|card| {
            card.classification == ModelClassification::Archetype
                && card.calibration_status == CalibrationStatus::Uncalibrated
                && !card.citations.is_empty()
        }));
    }
}

#[test]
fn unbound_rule_revision_and_out_of_window_definition_fail_closed() {
    let mut pack = ming_workshop_fixture();
    let gameplay = pack
        .model_cards
        .iter_mut()
        .find(|card| card.classification == ModelClassification::Synthetic)
        .expect("gameplay card");
    gameplay
        .rule_revisions
        .insert(RuleRevisionId::new("canwu.economy:rule:unbound-gameplay.v1").expect("ID"));
    gameplay.semantic_hash.clear();
    gameplay.semantic_hash = canwu_api::canonical_hash("canwu.economy.model-card.v1", gameplay)
        .expect("reseal test card");
    let error = compile_content_pack(&pack).expect_err("unbound rule revision must fail");
    assert!(error.message.contains("exactly bound"));

    let mut pack = ming_workshop_fixture();
    let key = pack
        .manifest
        .required_coverage_keys
        .iter()
        .next()
        .cloned()
        .expect("coverage key");
    for definition in &mut pack.definitions {
        definition.coverage_key.historical_years =
            Some(canwu_economy_reference_content::HistoricalYearWindowV1 {
                start_year: 1400,
                end_year_exclusive: 1645,
            });
        definition.semantic_hash.clear();
        definition.semantic_hash =
            canwu_api::canonical_hash("canwu.economy.behavior-definition.v1", definition)
                .expect("reseal test definition");
    }
    pack.manifest.required_coverage_keys.remove(&key);
    let mut expanded = key;
    expanded.historical_years = Some(canwu_economy_reference_content::HistoricalYearWindowV1 {
        start_year: 1400,
        end_year_exclusive: 1645,
    });
    pack.manifest.required_coverage_keys.insert(expanded);
    let error = compile_content_pack(&pack).expect_err("out-of-window definition must fail");
    assert!(error.message.contains("historical coverage exceeds"));
}
