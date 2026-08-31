#![allow(clippy::wildcard_imports)]

use crate::*;
use canwu_api::{SimTime, canonical_hash};
use canwu_production::ProcessRevisionId;
use canwu_resource::{ResourceDefinitionRevisionId, ResourceQualityId, ResourceUnitRevisionId};
use std::collections::BTreeSet;

#[must_use]
pub const fn fixture_ids() -> [&'static str; 3] {
    [
        "synthetic-grain",
        "ming-workshop",
        "china-industrialization",
    ]
}

#[must_use]
#[allow(clippy::too_many_lines)]
/// Builds the deterministic synthetic fixture used by content and integration tests.
///
/// # Panics
///
/// Panics only if a statically authored fixture identifier or duration violates the
/// corresponding validated identifier/time constructor. Such a panic is an authored
/// fixture defect rather than runtime input failure.
pub fn synthetic_grain_fixture() -> EconomyReferenceContentPackV1 {
    let period = period("canwu.economy:period:synthetic-fourteen-month");
    let region = region("canwu.economy:region:synthetic-river-basin");
    let grain = resource("canwu.resource:definition-revision:grain.synthetic.v1");
    let quality = quality("canwu.resource:quality:staple-grain.synthetic.v1");
    let unit = unit("canwu.resource:unit:grain-basket.synthetic.v1");
    let class = organization("canwu.economy:organization:granary-household.synthetic.v1");
    let mut card = synthetic_card(
        "canwu.economy:model-card:synthetic-grain-loop.v1",
        region.clone(),
        grain.clone(),
        quality.clone(),
        unit.clone(),
        "Gameplay calibration only; no historical population, yield, military, or price inference is permitted.",
    );
    for revision in [
        "canwu.economy:rule:seed-shortage-yield.v1",
        "canwu.economy:rule:missed-food-readiness.v1",
        "canwu.economy:rule:missed-fodder-readiness.v1",
        "canwu.economy:rule:high-throughput-fuel-shortage.v1",
        "canwu.economy:rule:high-throughput-ammunition-shortage.v1",
        "canwu.economy:rule:requisition-externality.v1",
        "canwu.economy:rule:route-bound-scarcity.v1",
    ] {
        card.rule_revisions
            .insert(RuleRevisionId::new(revision).expect("fixture ID"));
    }
    let fuel = resource("canwu.resource:definition-revision:fuel.synthetic.v1");
    let fuel_quality = crate::fixtures::quality("canwu.resource:quality:refined-fuel.synthetic.v1");
    let fuel_unit = crate::fixtures::unit("canwu.resource:unit:fuel-drum.synthetic.v1");
    card.resource_revisions.insert(fuel.clone());
    card.quality_revisions.insert(fuel_quality.clone());
    card.unit_revisions.insert(fuel_unit.clone());
    let fodder = resource("canwu.resource:definition-revision:fodder.synthetic.v1");
    let fodder_quality = crate::fixtures::quality("canwu.resource:quality:dry-fodder.synthetic.v1");
    let fodder_unit = crate::fixtures::unit("canwu.resource:unit:fodder-bale.synthetic.v1");
    let ammunition = resource("canwu.resource:definition-revision:ammunition.synthetic.v1");
    let ammunition_quality =
        crate::fixtures::quality("canwu.resource:quality:service-ammunition.synthetic.v1");
    let ammunition_unit = crate::fixtures::unit("canwu.resource:unit:ammunition-case.synthetic.v1");
    card.resource_revisions.insert(fodder.clone());
    card.quality_revisions.insert(fodder_quality.clone());
    card.unit_revisions.insert(fodder_unit.clone());
    card.resource_revisions.insert(ammunition.clone());
    card.quality_revisions.insert(ammunition_quality.clone());
    card.unit_revisions.insert(ammunition_unit.clone());
    reseal_model_card(&mut card);
    let harvest_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::SeasonalHarvest,
        grain.clone(),
        quality.clone(),
        unit.clone(),
        class.clone(),
        None,
    );
    let force_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::ForceSupply,
        grain.clone(),
        quality.clone(),
        unit.clone(),
        class.clone(),
        None,
    );
    let fodder_force_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::ForceSupply,
        fodder,
        fodder_quality,
        fodder_unit,
        class.clone(),
        None,
    );
    let requisition_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::RequisitionExternality,
        grain.clone(),
        quality.clone(),
        unit.clone(),
        class.clone(),
        None,
    );
    let high_throughput_force_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::ForceSupply,
        fuel,
        fuel_quality,
        fuel_unit,
        organization("canwu.economy:organization:high-throughput-motorized-force.synthetic.v1"),
        None,
    );
    let high_throughput_ammunition_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::ForceSupply,
        ammunition,
        ammunition_quality,
        ammunition_unit,
        organization("canwu.economy:organization:high-throughput-motorized-force.synthetic.v1"),
        None,
    );
    let scarcity_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::LocalScarcity,
        grain.clone(),
        quality.clone(),
        unit.clone(),
        class.clone(),
        None,
    );
    let price_key = key(
        period.clone(),
        region.clone(),
        EconomyMechanism::PricePressure,
        grain,
        quality,
        unit,
        class,
        None,
    );
    let harvest = definition(
        "canwu.economy:definition:synthetic-seasonal-harvest.v1",
        harvest_key.clone(),
        &card,
        &[
            ("harvest_output", 1_200, "grain_baskets"),
            ("seed_floor", 180, "grain_baskets"),
        ],
        &[(
            "canwu.economy:rule:seed-shortage-yield.v1",
            "output falls when the exact protected seed allocation is below the authored floor",
        )],
        None,
    );
    let force = definition(
        "canwu.economy:definition:synthetic-force-food.v1",
        force_key.clone(),
        &card,
        &[
            ("quantity_per_due", 40, "grain_baskets"),
            ("buffer_quantity", 80, "grain_baskets"),
            ("cadence_minutes", 1_440, "minutes"),
            ("resource_kind_code", 1, "force_supply_kind_v1"),
            ("shortage_tolerance_quantity", 0, "grain_baskets"),
            ("readiness_delta_per_mille", -100, "per_mille"),
            ("fatigue_delta_per_mille", 50, "per_mille"),
            ("cohesion_delta_per_mille", -20, "per_mille"),
            ("disease_delta_per_mille", 10, "per_mille"),
            ("desertion_delta_per_mille", 5, "per_mille"),
            ("nonlinear_or_threshold", 1, "boolean"),
        ],
        &[(
            "canwu.economy:rule:missed-food-readiness.v1",
            "a due food shortage produces a force-local readiness consequence on the next eligible boundary",
        )],
        None,
    );
    let high_throughput_force = definition(
        "canwu.economy:definition:synthetic-high-throughput-force-fuel.v1",
        high_throughput_force_key.clone(),
        &card,
        &[
            ("quantity_per_due", 120, "fuel_drums"),
            ("buffer_quantity", 360, "fuel_drums"),
            ("cadence_minutes", 360, "minutes"),
            ("resource_kind_code", 6, "force_supply_kind_v1"),
            ("shortage_tolerance_quantity", 10, "fuel_drums"),
            ("readiness_delta_per_mille", -160, "per_mille"),
            ("fatigue_delta_per_mille", 80, "per_mille"),
            ("cohesion_delta_per_mille", -25, "per_mille"),
            ("disease_delta_per_mille", 0, "per_mille"),
            ("desertion_delta_per_mille", 3, "per_mille"),
            ("nonlinear_or_threshold", 1, "boolean"),
        ],
        &[(
            "canwu.economy:rule:high-throughput-fuel-shortage.v1",
            "a due fuel shortage produces a force-local consequence at the authored six-hour cadence",
        )],
        None,
    );
    let fodder_force = definition(
        "canwu.economy:definition:synthetic-force-fodder.v1",
        fodder_force_key.clone(),
        &card,
        &[
            ("quantity_per_due", 24, "fodder_bales"),
            ("buffer_quantity", 48, "fodder_bales"),
            ("cadence_minutes", 720, "minutes"),
            ("resource_kind_code", 2, "force_supply_kind_v1"),
            ("shortage_tolerance_quantity", 2, "fodder_bales"),
            ("readiness_delta_per_mille", -45, "per_mille"),
            ("fatigue_delta_per_mille", 30, "per_mille"),
            ("cohesion_delta_per_mille", -10, "per_mille"),
            ("disease_delta_per_mille", 2, "per_mille"),
            ("desertion_delta_per_mille", 1, "per_mille"),
            ("nonlinear_or_threshold", 0, "boolean"),
        ],
        &[(
            "canwu.economy:rule:missed-fodder-readiness.v1",
            "a due fodder shortage compounds preindustrial force fatigue at its authored cadence",
        )],
        None,
    );
    let high_throughput_ammunition = definition(
        "canwu.economy:definition:synthetic-high-throughput-force-ammunition.v1",
        high_throughput_ammunition_key.clone(),
        &card,
        &[
            ("quantity_per_due", 80, "ammunition_cases"),
            ("buffer_quantity", 240, "ammunition_cases"),
            ("cadence_minutes", 180, "minutes"),
            ("resource_kind_code", 4, "force_supply_kind_v1"),
            ("shortage_tolerance_quantity", 5, "ammunition_cases"),
            ("readiness_delta_per_mille", -120, "per_mille"),
            ("fatigue_delta_per_mille", 35, "per_mille"),
            ("cohesion_delta_per_mille", -20, "per_mille"),
            ("disease_delta_per_mille", 0, "per_mille"),
            ("desertion_delta_per_mille", 2, "per_mille"),
            ("nonlinear_or_threshold", 1, "boolean"),
        ],
        &[(
            "canwu.economy:rule:high-throughput-ammunition-shortage.v1",
            "a due ammunition shortage constrains a high-throughput force independently of fuel",
        )],
        None,
    );
    let requisition = definition(
        "canwu.economy:definition:synthetic-requisition.v1",
        requisition_key.clone(),
        &card,
        &[
            ("cooperation_cost_per_mille", 80, "per_mille"),
            ("next_harvest_input_cost_per_mille", 60, "per_mille"),
        ],
        &[(
            "canwu.economy:rule:requisition-externality.v1",
            "an applied requisition intent lowers civilian cooperation and a later harvest input",
        )],
        Some(ExternalityApplicability::Required),
    );
    let scarcity = definition(
        "canwu.economy:definition:synthetic-local-scarcity.v1",
        scarcity_key.clone(),
        &card,
        &[("buffer_target", 240, "grain_baskets")],
        &[(
            "canwu.economy:rule:route-bound-scarcity.v1",
            "distant stock without a workable observed route does not reduce local scarcity",
        )],
        None,
    );
    let behavior: BTreeSet<DefinitionId> = [
        &harvest,
        &force,
        &fodder_force,
        &high_throughput_force,
        &high_throughput_ammunition,
        &requisition,
        &scarcity,
    ]
    .into_iter()
    .map(|value| value.id.clone())
    .collect();
    let all_behavior_cards = [card.id.clone()].into_iter().collect();
    EconomyReferenceContentPackV1 {
        manifest: manifest([
            harvest_key.clone(),
            force_key.clone(),
            fodder_force_key.clone(),
            high_throughput_force_key.clone(),
            high_throughput_ammunition_key.clone(),
            requisition_key.clone(),
            scarcity_key.clone(),
            price_key.clone(),
        ]),
        model_cards: vec![card.clone()],
        definitions: vec![harvest, force, fodder_force, high_throughput_force, high_throughput_ammunition, requisition, scarcity],
        coverage: vec![
            exact_coverage("canwu.economy:coverage-declaration:synthetic-behavior.v1", 100, [&harvest_key, &force_key, &fodder_force_key, &high_throughput_force_key, &high_throughput_ammunition_key, &requisition_key, &scarcity_key], behavior.clone(), all_behavior_cards),
            exact_non_behavioral("canwu.economy:coverage-declaration:synthetic-price-unknown.v1", 100, &price_key, CoverageStatus::ExplicitUnknown),
        ],
        profiles: vec![ReferenceProfileV1 {
            id: profile("canwu.economy:profile:synthetic-grain.v1"),
            label: "Synthetic fourteen-month grain and force-supply loop".to_owned(),
            historically_named: false,
            claims_calibrated: false,
            definition_ids: behavior,
            disclosures: vec![disclosure(&card, "all numeric fields and causal rules", "Every value is an explicit gameplay calibration and must not be presented as a historical estimate.")],
            design_note: "Exercises conservation, distinct daily preindustrial and six-hour higher-throughput force cadences, requisition externalities, and scarcity without a historical claim.".to_owned(),
        }],
    }
}

#[must_use]
#[allow(clippy::missing_panics_doc)]
#[allow(clippy::too_many_lines)]
pub fn ming_workshop_fixture() -> EconomyReferenceContentPackV1 {
    let period = period("canwu.economy:period:songjiang-cotton-1450-1644");
    let region = region("canwu.economy:region:songjiang-lower-yangzi");
    let input = resource("canwu.resource:definition-revision:raw-cotton.songjiang-archetype.v1");
    let quality = quality("canwu.resource:quality:cotton.songjiang-archetype.v1");
    let unit = unit("canwu.resource:unit:cotton-cloth-batch.songjiang-archetype.v1");
    let class =
        organization("canwu.economy:organization:songjiang-household-specialist-production.v1");
    let process = ProcessRevisionId::new("canwu.production:process:songjiang-household-cotton.v1")
        .expect("fixture ID");
    let historical_years = HistoricalYearWindowV1 {
        start_year: 1450,
        end_year_exclusive: 1645,
    };
    let mut factual_card = archetype_card(
        "canwu.economy:model-card:ming-workshop-organization.v1",
        region.clone(),
        input.clone(),
        quality.clone(),
        unit.clone(),
        Some(historical_years.clone()),
        "Harriet T. Zurndorfer, The Resistant Fibre: The Pre-modern History of Cotton in China",
        "https://www.lse.ac.uk/Economic-History/Assets/Documents/Research/GEHN/Padua/PADUAZurndorfer.pdf",
        "PDF pp. 4-7, especially the Songjiang discussion on pp. 5-7: household spinning and weaving, commercial raw-cotton and cloth networks, and merchant/broker distribution.",
        "The Songjiang evidence cannot establish a universal Ming workshop size, productivity, wage, factory-equivalent coefficient, or empire-wide organization.",
    );
    factual_card.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:zurndorfer-great-divergence.v1")
            .expect("fixture ID"),
        citation: "Harriet T. Zurndorfer, Cotton Textile Manufacture and Marketing in Late Imperial China and the 'Great Divergence'".to_owned(),
        url: "https://doi.org/10.1163/156852011X614028".to_owned(),
        locator: "Journal of the Economic and Social History of the Orient 54(5), 2011, pp. 701-738; pp. 713-718 discuss higher-grade and fancy cloth workshops, households specializing by operation, final dyeing and calendaring in large urban workshops, and broker-mediated circulation.".to_owned(),
    });
    factual_card.process_revisions.insert(process.clone());
    reseal_model_card(&mut factual_card);
    let mut gameplay_card = synthetic_card_with_years(
        "canwu.economy:model-card:ming-workshop-gameplay.v1",
        region.clone(),
        input.clone(),
        quality.clone(),
        unit.clone(),
        historical_years.clone(),
        "The numeric work-unit scale and executable household-specialization rule are gameplay calibration, not measured Ming quantities.",
    );
    gameplay_card.process_revisions.insert(process);
    gameplay_card.rule_revisions.insert(
        RuleRevisionId::new("canwu.economy:rule:household-specialist-production.v1")
            .expect("fixture ID"),
    );
    reseal_model_card(&mut gameplay_card);
    let key = key(
        period.clone(),
        region,
        EconomyMechanism::WorkshopProduction,
        input,
        quality,
        unit,
        class,
        Some(historical_years),
    );
    let factual = definition(
        "canwu.economy:definition:songjiang-household-cotton-facts.v1",
        key.clone(),
        &factual_card,
        &[],
        &[],
        None,
    );
    let workshop = definition(
        "canwu.economy:definition:songjiang-household-cotton.v1",
        key.clone(),
        &gameplay_card,
        &[("authored_work_units", 100, "work_units")],
        &[(
            "canwu.economy:rule:household-specialist-production.v1",
            "the Songjiang profile remains household production, including households specializing in particular stages, linked by brokers and merchants; workshops producing higher-grade or fancy cloth remain distinct from large urban workshops handling final dyeing and calendaring, and the system cannot be materialized as one anachronistic factory",
        )],
        None,
    );
    EconomyReferenceContentPackV1 {
        manifest: manifest([key.clone()]),
        model_cards: vec![factual_card.clone(), gameplay_card.clone()],
        definitions: vec![factual.clone(), workshop.clone()],
        coverage: vec![exact_coverage(
            "canwu.economy:coverage-declaration:ming-workshop-local.v1",
            200,
            [&key],
            [factual.id.clone(), workshop.id.clone()].into_iter().collect(),
            [factual_card.id.clone(), gameplay_card.id.clone()]
                .into_iter()
                .collect(),
        )],
        profiles: vec![ReferenceProfileV1 {
            id: profile("canwu.economy:profile:ming-workshop.v1"),
            label: "Ming-period Songjiang household and specialist-household cotton archetype".to_owned(),
            historically_named: true,
            claims_calibrated: false,
            definition_ids: [factual.id, workshop.id].into_iter().collect(),
            disclosures: vec![
                disclosure(&factual_card, "household, specialist-household, and distribution facts", "The organizational and geographic claims are source-linked but intentionally uncalibrated."),
                disclosure(&gameplay_card, "work units and executable organization rule", "The numeric work-unit scale and executable rule are synthetic cross-profile gameplay calibration, not a measured Ming productivity series."),
            ],
            design_note: "Scopes the fixture to Songjiang and the Lower Yangzi from the late fifteenth century through 1644, preserving household production, stage-specialized households, merchant/broker distribution, and separately evidenced urban finishing workshops instead of assuming a concentrated factory.".to_owned(),
        }],
    }
}

#[must_use]
#[allow(clippy::missing_panics_doc, clippy::too_many_lines)]
pub fn china_industrialization_fixture() -> EconomyReferenceContentPackV1 {
    let early_capability_period = period("canwu.economy:period:pingxiang-survey-and-land-1896");
    let operating_capability_period = period("canwu.economy:period:pingxiang-operating-mine-1904");
    let route_capability_period = period("canwu.economy:period:pingxiang-zhuzhou-route-1906-1907");
    let precursor_period =
        period("canwu.economy:period:hanyang-daye-pingxiang-precursors-1896-1907");
    let consolidated_period = period("canwu.economy:period:hanyeping-consolidated-1908-1911");
    let region = region("canwu.economy:region:middle-yangzi-hubei-jiangxi-industrial-chain");
    let coal = resource("canwu.resource:definition-revision:pingxiang-coal.hanyeping-archetype.v1");
    let quality = quality("canwu.resource:quality:pingxiang-coal.hanyeping-archetype.v1");
    let unit = unit("canwu.resource:unit:coal-tonne.hanyeping-archetype.v1");
    let precursor_class = organization(
        "canwu.economy:organization:hanyang-daye-pingxiang-linked-precursor-enterprises.v1",
    );
    let consolidated_class =
        organization("canwu.economy:organization:hanyeping-consolidated-company.v1");
    let precursor_years = HistoricalYearWindowV1 {
        start_year: 1896,
        end_year_exclusive: 1908,
    };
    let early_capability_years = HistoricalYearWindowV1 {
        start_year: 1896,
        end_year_exclusive: 1897,
    };
    let operating_capability_years = HistoricalYearWindowV1 {
        start_year: 1904,
        end_year_exclusive: 1905,
    };
    let route_capability_years = HistoricalYearWindowV1 {
        start_year: 1906,
        end_year_exclusive: 1908,
    };
    let consolidated_years = HistoricalYearWindowV1 {
        start_year: 1908,
        end_year_exclusive: 1912,
    };
    let early_capability_key = key(
        early_capability_period,
        region.clone(),
        EconomyMechanism::ResourceCapability,
        coal.clone(),
        quality.clone(),
        unit.clone(),
        precursor_class.clone(),
        Some(early_capability_years.clone()),
    );
    let operating_capability_key = key(
        operating_capability_period,
        region.clone(),
        EconomyMechanism::ResourceCapability,
        coal.clone(),
        quality.clone(),
        unit.clone(),
        precursor_class.clone(),
        Some(operating_capability_years.clone()),
    );
    let route_capability_key = key(
        route_capability_period,
        region.clone(),
        EconomyMechanism::ResourceCapability,
        coal.clone(),
        quality.clone(),
        unit.clone(),
        precursor_class.clone(),
        Some(route_capability_years.clone()),
    );
    let precursor_process_key = key(
        precursor_period,
        region.clone(),
        EconomyMechanism::IndustrialProduction,
        coal.clone(),
        quality.clone(),
        unit.clone(),
        precursor_class,
        Some(precursor_years.clone()),
    );
    let consolidated_capability_key = key(
        consolidated_period.clone(),
        region.clone(),
        EconomyMechanism::ResourceCapability,
        coal.clone(),
        quality.clone(),
        unit.clone(),
        consolidated_class.clone(),
        Some(consolidated_years.clone()),
    );
    let consolidated_process_key = key(
        consolidated_period,
        region.clone(),
        EconomyMechanism::IndustrialProduction,
        coal.clone(),
        quality.clone(),
        unit.clone(),
        consolidated_class,
        Some(consolidated_years.clone()),
    );
    let mut precursor_facts = archetype_card(
        "canwu.economy:model-card:hanyang-daye-pingxiang-precursor-facts.v1",
        region.clone(),
        coal.clone(),
        quality.clone(),
        unit.clone(),
        Some(precursor_years.clone()),
        "Jeff Hornibrook, Don't Tread On Me: Land, Officials, and Archival Work on a Qing Dynasty Mining Enterprise, Chinese Business History 15(1) (Spring 2005), 1-5",
        "https://ihss.hku.hk/wp-content/uploads/2020/09/cbh-15-1.pdf",
        "PDF p. 1, opening two paragraphs: the Pingxiang Coal Mine Bureau began acquiring mining and railway land in 1896. This establishes linked precursor activity and planning, not a completed route-accessible Hanyeping system.",
        "The Hubei-Jiangxi interregional industrial-chain evidence is not nationally representative and cannot imply inevitable productivity, one universal industrial path, or automatic technology diffusion.",
    );
    precursor_facts.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:liu-hanyeping-precursors.v1")
            .expect("fixture ID"),
        citation: "Yun Liu, Revisiting Hanyeping Company (1889–1908): A case study of China’s early industrialisation and corporate history, Business History 52(1) (2010), 62-73".to_owned(),
        url: "https://doi.org/10.1080/00076790903469612".to_owned(),
        locator: "Business History 52(1), pp. 62-73; pp. 62-64 identify the Hanyang, Daye, and Pingxiang precursor enterprises and frame the study through their 1908 consolidation.".to_owned(),
    });
    precursor_facts.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:perry-anyuan-modern-mine.v1")
            .expect("fixture ID"),
        citation: "Elizabeth J. Perry, Anyuan: Mining China's Revolutionary Tradition, University of California Press (2012)".to_owned(),
        url: "https://doi.org/10.1525/9780520954038".to_owned(),
        locator: "Book pp. 17-20: the modern enterprise was established in 1898; two mechanized horizontal adits were operating by 1904; after the Zhuzhou connection, coal moved by rail to Zhuzhou and onward by riverboat toward Hubei.".to_owned(),
    });
    precursor_facts.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:hornibrook-mechanized-mining.v1")
            .expect("fixture ID"),
        citation: "Jeff Hornibrook, Local Elites and Mechanized Mining in China: The Case of the Wen Lineage in Pingxiang County, Jiangxi, Modern China 27(2) (2001), 202-228".to_owned(),
        url: "https://doi.org/10.1177/009770040102700202".to_owned(),
        locator: "Modern China 27(2), pp. 202-228; documents mechanized mining and local elite participation through the Wen lineage in Pingxiang County, Jiangxi.".to_owned(),
    });
    precursor_facts.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:wu-empires-of-coal-pingxiang.v1")
            .expect("fixture ID"),
        citation: "Shellen Xiao Wu, Empires of Coal: Fueling China's Entry into the Modern World Order, 1860-1920".to_owned(),
        url: "https://doi.org/10.1515/9780804794732".to_owned(),
        locator: "Book pp. 117-118: before the railway connection, Pingxiang coal moved by water at about 30,000 tons per year; a journalist visited the operating mine in 1905.".to_owned(),
    });
    precursor_facts.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:wang-hunan-railway-chronology.v1")
            .expect("fixture ID"),
        citation: "Zhu Li, 'Why Is Zhuzhou a City Brought by Trains?' [株洲为何是“火车拉来的城市”？], Xin Hunan / Hunan Daily New Media, 2020-07-20".to_owned(),
        url: "https://m.voc.com.cn/xhn/news/202007/14436245.html".to_owned(),
        locator: "Railway-history paragraph: the line reached Yangsanshi, Liling, in 1903, was extended to Zhuzhou in 1905, and then transferred Pingxiang coal from rail to riverboat.".to_owned(),
    });
    let precursor_process = ProcessRevisionId::new(
        "canwu.production:process:hanyang-daye-pingxiang-linked-precursors.archetype.v1",
    )
    .expect("fixture ID");
    precursor_facts
        .process_revisions
        .insert(precursor_process.clone());
    reseal_model_card(&mut precursor_facts);
    let mut precursor_gameplay = synthetic_card_with_years(
        "canwu.economy:model-card:hanyang-daye-pingxiang-precursor-gameplay.v1",
        region.clone(),
        coal.clone(),
        quality.clone(),
        unit.clone(),
        precursor_years,
        "Pre-1908 industrial batch and maintenance values are scenario calibration for linked precursor enterprises, not historical measurements.",
    );
    precursor_gameplay
        .process_revisions
        .insert(precursor_process.clone());
    precursor_gameplay.rule_revisions.insert(
        RuleRevisionId::new("canwu.economy:rule:precursor-industrial-constraint-separation.v1")
            .expect("fixture ID"),
    );
    reseal_model_card(&mut precursor_gameplay);

    let mut consolidated_facts = archetype_card(
        "canwu.economy:model-card:hanyeping-consolidated-facts.v1",
        region.clone(),
        coal.clone(),
        quality.clone(),
        unit.clone(),
        Some(consolidated_years.clone()),
        "Yun Liu, Revisiting Hanyeping Company (1889–1908): A case study of China’s early industrialisation and corporate history, Business History 52(1) (2010), 62-73",
        "https://doi.org/10.1080/00076790903469612",
        "Business History 52(1), pp. 62-73; pp. 62-64 identify the constituent enterprises and their 1908 consolidation. The consolidated class is therefore effective no earlier than 1908.",
        "The Hanyeping evidence cannot establish a universal Chinese industrial trajectory, a national productivity series, or automatic route performance.",
    );
    consolidated_facts.citations.push(CitationV1 {
        id: CitationId::new(
            "canwu.economy:citation:hornibrook-mechanized-mining-consolidated.v1",
        )
        .expect("fixture ID"),
        citation: "Jeff Hornibrook, Local Elites and Mechanized Mining in China: The Case of the Wen Lineage in Pingxiang County, Jiangxi, Modern China 27(2) (2001), 202-228".to_owned(),
        url: "https://doi.org/10.1177/009770040102700202".to_owned(),
        locator: "Modern China 27(2), pp. 202-228; supports the Pingxiang mechanized-mining and local-elite context independently of the 1908 merger date.".to_owned(),
    });
    consolidated_facts.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:perry-anyuan-route-consolidated.v1")
            .expect("fixture ID"),
        citation: "Elizabeth J. Perry, Anyuan: Mining China's Revolutionary Tradition, University of California Press (2012)".to_owned(),
        url: "https://doi.org/10.1525/9780520954038".to_owned(),
        locator: "Book pp. 19-20: after the Zhuzhou connection, coal moved by rail to Zhuzhou and onward by riverboat toward Hubei.".to_owned(),
    });
    consolidated_facts.citations.push(CitationV1 {
        id: CitationId::new(
            "canwu.economy:citation:wang-hunan-railway-chronology-consolidated.v1",
        )
        .expect("fixture ID"),
        citation: "Zhu Li, 'Why Is Zhuzhou a City Brought by Trains?' [株洲为何是“火车拉来的城市”？], Xin Hunan / Hunan Daily New Media, 2020-07-20".to_owned(),
        url: "https://m.voc.com.cn/xhn/news/202007/14436245.html".to_owned(),
        locator: "Railway-history paragraph: the line reached Yangsanshi, Liling, in 1903, was extended to Zhuzhou in 1905, and then transferred Pingxiang coal from rail to riverboat.".to_owned(),
    });
    consolidated_facts.citations.push(CitationV1 {
        id: CitationId::new("canwu.economy:citation:wu-empires-of-coal-consolidated.v1")
            .expect("fixture ID"),
        citation: "Shellen Xiao Wu, Empires of Coal: Fueling China's Entry into the Modern World Order, 1860-1920".to_owned(),
        url: "https://doi.org/10.1515/9780804794732".to_owned(),
        locator: "Book pp. 117-118: documents water transport before the railway connection and an operating Pingxiang mine visited in 1905.".to_owned(),
    });
    let consolidated_process = ProcessRevisionId::new(
        "canwu.production:process:hanyeping-consolidated-coal-iron-chain.archetype.v1",
    )
    .expect("fixture ID");
    consolidated_facts
        .process_revisions
        .insert(consolidated_process.clone());
    reseal_model_card(&mut consolidated_facts);
    let mut consolidated_gameplay = synthetic_card_with_years(
        "canwu.economy:model-card:hanyeping-consolidated-gameplay.v1",
        region,
        coal,
        quality,
        unit,
        consolidated_years,
        "Post-1908 output batches, spares floors, and executable shortage rules are scenario calibration rather than values derived from Liu's article.",
    );
    consolidated_gameplay
        .process_revisions
        .insert(consolidated_process.clone());
    consolidated_gameplay.rule_revisions.insert(
        RuleRevisionId::new("canwu.economy:rule:consolidated-industrial-constraint-separation.v1")
            .expect("fixture ID"),
    );
    reseal_model_card(&mut consolidated_gameplay);

    let early_capability = ResourceCapabilityRevision {
        id: ResourceCapabilityRevisionId::new(
            "canwu.economy:capability:pingxiang-coal-survey-and-land-1896.v1",
        )
        .expect("fixture ID"),
        definition_id: DefinitionId::new(
            "canwu.economy:definition:pingxiang-coal-survey-and-land-capability.v1",
        )
        .expect("fixture ID"),
        coverage_key: early_capability_key.clone(),
        stage: ResourceCapabilityStage::ObservedSurveyed,
        effective_period: EffectivePeriodV1 {
            start: SimTime::from_minutes(-1_000_000),
            end: SimTime::from_minutes(1_000_000),
        },
        surveyed_or_operating_site: Some(
            "Pingxiang Coal Mine Bureau land and railway acquisition program in Jiangxi".to_owned(),
        ),
        suitable_process_revisions: [precursor_process.clone()].into_iter().collect(),
        route_access_evidence_class: None,
        model_card_ids: [precursor_facts.id.clone()].into_iter().collect(),
    };
    let early_capability_definition = definition_with_capability(
        "canwu.economy:definition:pingxiang-coal-survey-and-land-capability.v1",
        early_capability_key.clone(),
        &precursor_facts,
        early_capability,
    );
    let operating_capability = ResourceCapabilityRevision {
        id: ResourceCapabilityRevisionId::new(
            "canwu.economy:capability:pingxiang-coal-operating-mine-1904.v1",
        )
        .expect("fixture ID"),
        definition_id: DefinitionId::new(
            "canwu.economy:definition:pingxiang-coal-operating-mine-capability.v1",
        )
        .expect("fixture ID"),
        coverage_key: operating_capability_key.clone(),
        stage: ResourceCapabilityStage::OperatingSite,
        effective_period: EffectivePeriodV1 {
            start: SimTime::from_minutes(-1_000_000),
            end: SimTime::from_minutes(1_000_000),
        },
        surveyed_or_operating_site: Some(
            "Two mechanized horizontal adits operating at the Pingxiang coal mine in Jiangxi by 1904".to_owned(),
        ),
        suitable_process_revisions: [precursor_process.clone()].into_iter().collect(),
        route_access_evidence_class: None,
        model_card_ids: [precursor_facts.id.clone()].into_iter().collect(),
    };
    let operating_capability_definition = definition_with_capability(
        "canwu.economy:definition:pingxiang-coal-operating-mine-capability.v1",
        operating_capability_key.clone(),
        &precursor_facts,
        operating_capability,
    );
    let route_capability = ResourceCapabilityRevision {
        id: ResourceCapabilityRevisionId::new(
            "canwu.economy:capability:pingxiang-coal-zhuzhou-route-full-year-1906-1907.v1",
        )
        .expect("fixture ID"),
        definition_id: DefinitionId::new(
            "canwu.economy:definition:pingxiang-coal-zhuzhou-route-capability.v1",
        )
        .expect("fixture ID"),
        coverage_key: route_capability_key.clone(),
        stage: ResourceCapabilityStage::RouteAccessible,
        effective_period: EffectivePeriodV1 {
            start: SimTime::from_minutes(-1_000_000),
            end: SimTime::from_minutes(1_000_000),
        },
        surveyed_or_operating_site: Some(
            "Pingxiang coal routed by rail to Zhuzhou and onward by riverboat toward Hubei; 1906 is the first whole-year proxy after the 1905 extension".to_owned(),
        ),
        suitable_process_revisions: [precursor_process].into_iter().collect(),
        route_access_evidence_class: Some(
            "source-bounded rail-to-Zhuzhou and riverboat-to-Hubei chain; exact scenario route observation and destination acceptance remain required at runtime".to_owned(),
        ),
        model_card_ids: [precursor_facts.id.clone()].into_iter().collect(),
    };
    let route_capability_definition = definition_with_capability(
        "canwu.economy:definition:pingxiang-coal-zhuzhou-route-capability.v1",
        route_capability_key.clone(),
        &precursor_facts,
        route_capability,
    );
    let precursor_factual_definition = definition(
        "canwu.economy:definition:hanyang-daye-pingxiang-linked-precursor-facts.v1",
        precursor_process_key.clone(),
        &precursor_facts,
        &[],
        &[],
        None,
    );
    let precursor_industrial = definition(
        "canwu.economy:definition:hanyang-daye-pingxiang-linked-precursor-gameplay.v1",
        precursor_process_key.clone(),
        &precursor_gameplay,
        &[
            ("authored_output_batch", 60, "industrial_output_units"),
            ("maintenance_spares_floor", 10, "spares_units"),
        ],
        &[(
            "canwu.economy:rule:precursor-industrial-constraint-separation.v1",
            "linked precursor enterprises remain separate and idle when exact fuel quality, transport, maintenance, skills, finance, or organization evidence is absent",
        )],
        None,
    );

    let consolidated_capability = ResourceCapabilityRevision {
        id: ResourceCapabilityRevisionId::new(
            "canwu.economy:capability:hanyeping-pingxiang-coal-route-accessible-1908.v1",
        )
        .expect("fixture ID"),
        definition_id: DefinitionId::new(
            "canwu.economy:definition:hanyeping-consolidated-coal-capability.v1",
        )
        .expect("fixture ID"),
        coverage_key: consolidated_capability_key.clone(),
        stage: ResourceCapabilityStage::RouteAccessible,
        effective_period: EffectivePeriodV1 {
            start: SimTime::from_minutes(-1_000_000),
            end: SimTime::from_minutes(1_000_000),
        },
        surveyed_or_operating_site: Some(
            "1908-consolidated Hanyang works, Daye iron mine, and Pingxiang coal enterprise"
                .to_owned(),
        ),
        suitable_process_revisions: [consolidated_process].into_iter().collect(),
        route_access_evidence_class: Some(
            "the transport-capability evidence predates the 1908 corporate consolidation; exact effective route observation and delivered acceptance remain required at runtime"
                .to_owned(),
        ),
        model_card_ids: [consolidated_facts.id.clone()].into_iter().collect(),
    };
    let consolidated_capability_definition = definition_with_capability(
        "canwu.economy:definition:hanyeping-consolidated-coal-capability.v1",
        consolidated_capability_key.clone(),
        &consolidated_facts,
        consolidated_capability,
    );
    let consolidated_factual_definition = definition(
        "canwu.economy:definition:hanyeping-consolidated-company-facts.v1",
        consolidated_process_key.clone(),
        &consolidated_facts,
        &[],
        &[],
        None,
    );
    let consolidated_industrial = definition(
        "canwu.economy:definition:hanyeping-consolidated-gameplay.v1",
        consolidated_process_key.clone(),
        &consolidated_gameplay,
        &[
            ("authored_output_batch", 100, "industrial_output_units"),
            ("maintenance_spares_floor", 12, "spares_units"),
        ],
        &[(
            "canwu.economy:rule:consolidated-industrial-constraint-separation.v1",
            "the plant idles when exact fuel quality, transport, maintenance, skilled personnel, finance or organization evidence is absent",
        )],
        None,
    );
    let precursor_definitions: BTreeSet<DefinitionId> = [
        early_capability_definition.id.clone(),
        operating_capability_definition.id.clone(),
        route_capability_definition.id.clone(),
        precursor_factual_definition.id.clone(),
        precursor_industrial.id.clone(),
    ]
    .into_iter()
    .collect();
    let consolidated_definitions: BTreeSet<DefinitionId> = [
        consolidated_capability_definition.id.clone(),
        consolidated_factual_definition.id.clone(),
        consolidated_industrial.id.clone(),
    ]
    .into_iter()
    .collect();
    EconomyReferenceContentPackV1 {
        manifest: manifest([
            early_capability_key.clone(),
            operating_capability_key.clone(),
            route_capability_key.clone(),
            precursor_process_key.clone(),
            consolidated_capability_key.clone(),
            consolidated_process_key.clone(),
        ]),
        model_cards: vec![
            precursor_facts.clone(),
            precursor_gameplay.clone(),
            consolidated_facts.clone(),
            consolidated_gameplay.clone(),
        ],
        definitions: vec![
            early_capability_definition,
            operating_capability_definition,
            route_capability_definition,
            precursor_factual_definition,
            precursor_industrial,
            consolidated_capability_definition,
            consolidated_factual_definition,
            consolidated_industrial,
        ],
        coverage: vec![
            exact_coverage(
                "canwu.economy:coverage-declaration:china-industrial-precursors.v1",
                200,
                [
                    &early_capability_key,
                    &operating_capability_key,
                    &route_capability_key,
                    &precursor_process_key,
                ],
                precursor_definitions.clone(),
                [precursor_facts.id.clone(), precursor_gameplay.id.clone()]
                    .into_iter()
                    .collect(),
            ),
            exact_coverage(
                "canwu.economy:coverage-declaration:hanyeping-consolidated.v1",
                200,
                [&consolidated_capability_key, &consolidated_process_key],
                consolidated_definitions.clone(),
                [consolidated_facts.id.clone(), consolidated_gameplay.id.clone()]
                    .into_iter()
                    .collect(),
            ),
        ],
        profiles: vec![
            ReferenceProfileV1 {
                id: profile("canwu.economy:profile:china-industrial-precursors.v1"),
                label: "Linked Hanyang-Daye-Pingxiang precursor enterprises, 1896-1907"
                    .to_owned(),
                historically_named: true,
                claims_calibrated: false,
                definition_ids: precursor_definitions,
                disclosures: vec![
                    disclosure(&precursor_facts, "precursor enterprise, land-acquisition, operating-mine, and transport facts", "The sources support separate linked precursor enterprises, an 1896 land-acquisition observation, mechanized adits operating by 1904, and the rail-to-Zhuzhou plus riverboat-to-Hubei chain after the 1905 extension; they do not imply continuous capability between evidence nodes, an integrated Hanyeping company before 1908, or automatic runtime delivery."),
                    disclosure(&precursor_gameplay, "precursor output, spares, and executable constraint rule", "All numeric and executable fields are uncalibrated gameplay values."),
                ],
                design_note: "Keeps Hanyang, Daye, and Pingxiang as linked precursor enterprises before 1908 while separating the 1896 survey/land observation, the 1904 operating-mine node, and a 1906 first-whole-year proxy for the rail-to-Zhuzhou and riverboat-to-Hubei chain completed during 1905; unsupported intervening capability remains unspecified, and corporate consolidation never creates transport capability by itself.".to_owned(),
            },
            ReferenceProfileV1 {
                id: profile("canwu.economy:profile:hanyeping-consolidated.v1"),
                label: "Consolidated Hanyeping company archetype, 1908-1911".to_owned(),
                historically_named: true,
                claims_calibrated: false,
                definition_ids: consolidated_definitions,
                disclosures: vec![
                    disclosure(&consolidated_facts, "1908 consolidated-company chronology and independently sourced transport context", "The named integrated class begins in 1908, but operating and transport evidence predates the merger; runtime route and delivery evidence remain separate."),
                    disclosure(&consolidated_gameplay, "industrial batch, spares floor, and executable constraint rule", "All numeric and executable fields are uncalibrated gameplay values, not a historical production series."),
                ],
                design_note: "Begins the integrated Hanyeping organization in 1908 without treating the merger as the origin of transport capability, keeps route access separate from delivered acceptance, and preserves quality, maintenance, skill, finance, and organization blockers.".to_owned(),
            },
        ],
    }
}

fn manifest<const N: usize>(keys: [CoverageKeyV1; N]) -> ContentManifestV1 {
    ContentManifestV1 {
        schema_version: CONTENT_SCHEMA_VERSION,
        pack_id: ContentPackId::new(PACK_ID).expect("fixture ID"),
        pack_version: env!("CARGO_PKG_VERSION").to_owned(),
        license: "Apache-2.0".to_owned(),
        required_coverage_keys: keys.into_iter().collect(),
    }
}

fn synthetic_card(
    id: &str,
    region: RegionId,
    resource: ResourceDefinitionRevisionId,
    quality: ResourceQualityId,
    unit: ResourceUnitRevisionId,
    forbidden: &str,
) -> ModelCardV1 {
    make_card(
        id,
        ModelClassification::Synthetic,
        region,
        resource,
        quality,
        unit,
        None,
        Vec::new(),
        forbidden,
        CalibrationStatus::Uncalibrated,
    )
}

#[allow(clippy::too_many_arguments)]
fn synthetic_card_with_years(
    id: &str,
    region: RegionId,
    resource: ResourceDefinitionRevisionId,
    quality: ResourceQualityId,
    unit: ResourceUnitRevisionId,
    historical_years: HistoricalYearWindowV1,
    forbidden: &str,
) -> ModelCardV1 {
    make_card(
        id,
        ModelClassification::Synthetic,
        region,
        resource,
        quality,
        unit,
        Some(historical_years),
        Vec::new(),
        forbidden,
        CalibrationStatus::Uncalibrated,
    )
}

#[allow(clippy::too_many_arguments)]
fn archetype_card(
    id: &str,
    region: RegionId,
    resource: ResourceDefinitionRevisionId,
    quality: ResourceQualityId,
    unit: ResourceUnitRevisionId,
    historical_years: Option<HistoricalYearWindowV1>,
    citation: &str,
    url: &str,
    locator: &str,
    forbidden: &str,
) -> ModelCardV1 {
    make_card(
        id,
        ModelClassification::Archetype,
        region,
        resource,
        quality,
        unit,
        historical_years,
        vec![CitationV1 {
            id: CitationId::new(format!("{id}:citation:1")).expect("fixture ID"),
            citation: citation.to_owned(),
            url: url.to_owned(),
            locator: locator.to_owned(),
        }],
        forbidden,
        CalibrationStatus::Uncalibrated,
    )
}

#[allow(clippy::too_many_arguments)]
fn make_card(
    id: &str,
    classification: ModelClassification,
    region: RegionId,
    resource: ResourceDefinitionRevisionId,
    quality: ResourceQualityId,
    unit: ResourceUnitRevisionId,
    historical_years: Option<HistoricalYearWindowV1>,
    citations: Vec<CitationV1>,
    forbidden: &str,
    calibration_status: CalibrationStatus,
) -> ModelCardV1 {
    let mut card = ModelCardV1 {
        id: ModelCardId::new(id).expect("fixture ID"),
        classification,
        citations,
        claim_scope: "Bounded reference-profile calibration and structural comparison only.".to_owned(),
        forbidden_inferences: vec![forbidden.to_owned()],
        competing_interpretations: vec!["Alternative regional, institutional, technological, and source interpretations remain possible.".to_owned()],
        geographic_scope: [region].into_iter().collect(),
        historical_years,
        effective_period: EffectivePeriodV1 { start: SimTime::from_minutes(-1_000_000), end: SimTime::from_minutes(1_000_000) },
        resource_revisions: [resource].into_iter().collect(),
        unit_revisions: [unit].into_iter().collect(),
        quality_revisions: [quality].into_iter().collect(),
        process_revisions: BTreeSet::new(),
        rule_revisions: BTreeSet::new(),
        extraction_or_conversion_derivation: "Integers are stored in exact authored fixture units; no implicit unit or historical conversion is performed.".to_owned(),
        uncertainty: Some(UncertaintyIntervalV1 { low: 0, high: 1_000, unit: "authored_scale".to_owned() }),
        confidence: if classification == ModelClassification::Synthetic { ConfidenceLevel::Low } else { ConfidenceLevel::Medium },
        calibration_status,
        semantic_hash: String::new(),
    };
    card.semantic_hash =
        canonical_hash("canwu.economy.model-card.v1", &card).expect("fixture encoding");
    card
}

fn reseal_model_card(card: &mut ModelCardV1) {
    card.semantic_hash.clear();
    card.semantic_hash =
        canonical_hash("canwu.economy.model-card.v1", card).expect("fixture encoding");
}

fn definition(
    id: &str,
    key: CoverageKeyV1,
    card: &ModelCardV1,
    fields: &[(&str, i64, &str)],
    rules: &[(&str, &str)],
    externality: Option<ExternalityApplicability>,
) -> BehavioralDefinitionV1 {
    let mut definition = BehavioralDefinitionV1 {
        id: DefinitionId::new(id).expect("fixture ID"),
        mechanism: key.mechanism,
        coverage_key: key,
        numeric_fields: fields
            .iter()
            .map(|(field, value, unit)| AuthoredValueV1 {
                field: (*field).to_owned(),
                value: *value,
                unit: (*unit).to_owned(),
                origin: AuthoredValueOrigin::GameplayCalibration,
                derivation: "Exact scenario-authored gameplay value; no historical conversion or empirical derivation is claimed.".to_owned(),
                model_card_id: card.id.clone(),
            })
            .collect(),
        causal_rules: rules
            .iter()
            .map(|(revision, rule)| AuthoredRuleV1 {
                rule_revision: RuleRevisionId::new(*revision).expect("fixture ID"),
                rule: (*rule).to_owned(),
                nature: AuthoredRuleNature::GameplayRule,
                model_card_id: card.id.clone(),
            })
            .collect(),
        resource_capability: None,
        externality_applicability: externality,
        model_card_ids: [card.id.clone()].into_iter().collect(),
        semantic_hash: String::new(),
    };
    definition.semantic_hash = canonical_hash("canwu.economy.behavior-definition.v1", &definition)
        .expect("fixture encoding");
    definition
}

fn definition_with_capability(
    id: &str,
    key: CoverageKeyV1,
    card: &ModelCardV1,
    capability: ResourceCapabilityRevision,
) -> BehavioralDefinitionV1 {
    let mut definition = definition(id, key, card, &[], &[], None);
    definition.resource_capability = Some(capability);
    definition.semantic_hash = String::new();
    definition.semantic_hash = canonical_hash("canwu.economy.behavior-definition.v1", &definition)
        .expect("fixture encoding");
    definition
}

fn exact_coverage<const N: usize>(
    id: &str,
    priority: u16,
    keys: [&CoverageKeyV1; N],
    definition_ids: BTreeSet<DefinitionId>,
    model_card_ids: BTreeSet<ModelCardId>,
) -> CoverageDeclarationV1 {
    CoverageDeclarationV1 {
        id: CoverageDeclarationId::new(id).expect("fixture ID"),
        priority,
        selector: selector(keys),
        status: CoverageStatus::ArchetypeFallback,
        definition_ids,
        model_card_ids,
    }
}

fn exact_non_behavioral(
    id: &str,
    priority: u16,
    key: &CoverageKeyV1,
    status: CoverageStatus,
) -> CoverageDeclarationV1 {
    CoverageDeclarationV1 {
        id: CoverageDeclarationId::new(id).expect("fixture ID"),
        priority,
        selector: selector([key]),
        status,
        definition_ids: BTreeSet::new(),
        model_card_ids: BTreeSet::new(),
    }
}

fn selector<const N: usize>(keys: [&CoverageKeyV1; N]) -> CoverageSelectorV1 {
    CoverageSelectorV1 {
        periods: keys.iter().map(|key| key.period.clone()).collect(),
        regions: keys.iter().map(|key| key.region.clone()).collect(),
        mechanisms: keys.iter().map(|key| key.mechanism).collect(),
        resource_revisions: keys
            .iter()
            .map(|key| key.resource_revision.clone())
            .collect(),
        quality_revisions: keys
            .iter()
            .map(|key| key.quality_revision.clone())
            .collect(),
        unit_revisions: keys.iter().map(|key| key.unit_revision.clone()).collect(),
        process_or_organization_classes: keys
            .iter()
            .map(|key| key.process_or_organization_class.clone())
            .collect(),
    }
}

fn disclosure(card: &ModelCardV1, field: &str, text: &str) -> ProfileDisclosureV1 {
    ProfileDisclosureV1 {
        field_or_rule: field.to_owned(),
        classification: card.classification,
        model_card_id: card.id.clone(),
        disclosure: text.to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn key(
    period: PeriodId,
    region: RegionId,
    mechanism: EconomyMechanism,
    resource_revision: ResourceDefinitionRevisionId,
    quality_revision: ResourceQualityId,
    unit_revision: ResourceUnitRevisionId,
    process_or_organization_class: OrganizationClassId,
    historical_years: Option<HistoricalYearWindowV1>,
) -> CoverageKeyV1 {
    CoverageKeyV1 {
        period,
        historical_years,
        region,
        mechanism,
        resource_revision,
        quality_revision,
        unit_revision,
        process_or_organization_class,
    }
}

fn period(value: &str) -> PeriodId {
    PeriodId::new(value).expect("fixture ID")
}
fn region(value: &str) -> RegionId {
    RegionId::new(value).expect("fixture ID")
}
fn profile(value: &str) -> ProfileId {
    ProfileId::new(value).expect("fixture ID")
}
fn organization(value: &str) -> OrganizationClassId {
    OrganizationClassId::new(value).expect("fixture ID")
}
fn resource(value: &str) -> ResourceDefinitionRevisionId {
    ResourceDefinitionRevisionId::new(value).expect("fixture ID")
}
fn quality(value: &str) -> ResourceQualityId {
    ResourceQualityId::new(value).expect("fixture ID")
}
fn unit(value: &str) -> ResourceUnitRevisionId {
    ResourceUnitRevisionId::new(value).expect("fixture ID")
}
