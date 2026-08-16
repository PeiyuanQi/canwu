#![allow(clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, EntityRef, ErrorCode, ReservationOffer,
    ReservationPoolKey, ReservationRef, ReservationRequest, SimulationPlugin, SimulationView,
    StateKey, StateVisibility, SystemCadence, TerritoryId,
};
use serde_json::Value;

fn grain_pool() -> ReservationPoolKey {
    ReservationPoolKey::new(
        StateKey::new("logistics", "grain"),
        EntityRef::Territory(TerritoryId::new(1)),
        "grain",
    )
}

fn offer_grain(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        offers: vec![ReservationOffer {
            pool: grain_pool(),
            capacity: 10,
        }],
        ..BoundaryProposal::default()
    })
}

fn request_grain(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        requests: vec![ReservationRequest {
            request: "daily-grain".to_owned(),
            pool: grain_pool(),
            quantity: 6,
            priority: 10,
            tie_break: "western-garrison".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn apply_grain(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let reservation = ReservationRef::new("example-demand", "request", "daily-grain");
    let allocation = view.reservation(&reservation)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidBoundary,
            "the declared grain reservation did not produce allocation evidence",
        )
    })?;
    let territory = EntityRef::Territory(TerritoryId::new(1));
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::SetComponent {
                state: StateKey::new("garrison", "grain"),
                entity: territory.clone(),
                component: "daily_grant".to_owned(),
                value: Value::from(allocation.granted),
                summary: format!(
                    "Granted {} grain to the western garrison",
                    allocation.granted
                ),
            },
            BoundaryDirective::Emit {
                event_type: "grain_allocated".to_owned(),
                summary: "The daily grain allocation settled".to_owned(),
                affected: vec![territory],
            },
        ],
        ..BoundaryProposal::default()
    })
}

struct SupplyPlugin;

impl SimulationPlugin for SupplyPlugin {
    fn name(&self) -> &'static str {
        "example-supply"
    }

    fn register(&self, registrar: &mut canwu_api::PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "offer",
            BoundaryPhase::ReservationAndAllocation,
            SystemCadence::Daily,
        );
        contract.reservation_offers = vec![StateKey::new("logistics", "grain")];
        registrar.register_boundary_system(contract, offer_grain)
    }
}

struct DemandPlugin;

impl SimulationPlugin for DemandPlugin {
    fn name(&self) -> &'static str {
        "example-demand"
    }

    fn register(&self, registrar: &mut canwu_api::PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut request = BoundarySystemContract::new(
            "request",
            BoundaryPhase::ReservationAndAllocation,
            SystemCadence::Daily,
        );
        request.reservation_requests = vec![StateKey::new("logistics", "grain")];
        registrar.register_boundary_system(request, request_grain)?;

        let mut apply = BoundarySystemContract::new(
            "apply",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        apply.writes = vec![StateKey::new("garrison", "grain")];
        apply.emits = vec!["grain_allocated".to_owned()];
        apply.reservation_reads = vec![ReservationRef::new(
            "example-demand",
            "request",
            "daily-grain",
        )];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, apply_grain)
    }
}

fn main() -> Result<(), CanwuError> {
    let mut canwu = Canwu::demo(35)?;
    canwu.register_plugin(&SupplyPlugin)?;
    canwu.register_plugin(&DemandPlugin)?;

    let receipt = canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()).with_cadence(SystemCadence::Daily))?;
    assert_eq!(receipt.allocations[0].granted, 6);
    assert_eq!(canwu.boundaries()[0].emissions.len(), 2);
    Ok(())
}
