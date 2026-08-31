use canwu_api::SimTime;
use canwu_production::{
    ProcessRevision, ProcessRevisionId, ProductionOutputSpec, ProductionRequirementAlternative,
    ProductionRequirementGroup, ProductionRequirementKind, ProductionState, ResourceRequirement,
};
use canwu_resource::{ResourceDefinitionRevisionId, ResourceUnitRevisionId};

fn main() {
    let unit = ResourceUnitRevisionId::new("resource:mass-unit:v1").expect("unit ID");
    let process = ProcessRevision {
        id: ProcessRevisionId::new("production:steam-mill:v1").expect("process ID"),
        label: "machine and fuel mill".to_owned(),
        semantic_digest: "example-process-digest".to_owned(),
        effective_from: SimTime::EPOCH,
        effective_until: None,
        work_units: 20,
        requirements: vec![
            requirement(
                "machine",
                ProductionRequirementKind::ToolsMachines,
                "steam-mill",
                1,
            ),
            requirement(
                "fuel",
                ProductionRequirementKind::Energy,
                "coal-grade-a",
                10,
            ),
            requirement(
                "maintenance",
                ProductionRequirementKind::Maintenance,
                "mill-maintained",
                1,
            ),
        ],
        inputs: vec![ResourceRequirement {
            resource: ResourceDefinitionRevisionId::new("resource:grain:v1").expect("resource ID"),
            unit: unit.clone(),
            quantity: 10,
        }],
        outputs: vec![ProductionOutputSpec {
            resource: ResourceDefinitionRevisionId::new("resource:flour:v1").expect("resource ID"),
            unit,
            quantity: 8,
            quality_class: "ordinary".to_owned(),
        }],
        capacity: Vec::new(),
        adoption_required: true,
    };
    let state = ProductionState::default();
    for blocker in state.blockers_for(&process, &[]) {
        println!("{:?}: {}", blocker.kind, blocker.next_eligible_action);
    }
}

fn requirement(
    id: &str,
    kind: ProductionRequirementKind,
    capability: &str,
    minimum_quantity: u64,
) -> ProductionRequirementGroup {
    ProductionRequirementGroup {
        id: format!("production:requirement:{id}"),
        kind,
        any_of: vec![ProductionRequirementAlternative {
            id: format!("production:requirement-alternative:{id}:primary"),
            capability: capability.to_owned(),
            minimum_quantity,
        }],
    }
}
