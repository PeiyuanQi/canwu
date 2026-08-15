//! Historical entity read models and detached world snapshots.

use canwu_core::{ArmyId, GovernmentId, PersonId, RouteId, TerritoryId};
use canwu_time::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct MapPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Person {
    pub id: PersonId,
    pub name: String,
    pub government: GovernmentId,
    pub current_location: TerritoryId,
    pub roles: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Government {
    pub id: GovernmentId,
    pub name: String,
    pub capital: TerritoryId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Territory {
    pub id: TerritoryId,
    pub name: String,
    pub controller: GovernmentId,
    pub position: MapPoint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Route {
    pub id: RouteId,
    pub name: String,
    pub from: TerritoryId,
    pub to: TerritoryId,
    pub travel_minutes: i64,
    pub terrain: String,
}

impl Route {
    #[must_use]
    pub fn connects(&self, first: TerritoryId, second: TerritoryId) -> bool {
        (self.from == first && self.to == second) || (self.from == second && self.to == first)
    }

    #[must_use]
    pub fn other_end(&self, territory: TerritoryId) -> Option<TerritoryId> {
        if self.from == territory {
            Some(self.to)
        } else if self.to == territory {
            Some(self.from)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitState {
    pub from: TerritoryId,
    pub to: TerritoryId,
    pub departed_at: SimTime,
    pub arrives_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Army {
    pub id: ArmyId,
    pub name: String,
    pub government: GovernmentId,
    pub commander: PersonId,
    pub location: TerritoryId,
    pub strength: u32,
    pub morale: u16,
    pub transit: Option<TransitState>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorldSnapshot {
    pub people: Vec<Person>,
    pub governments: Vec<Government>,
    pub territories: Vec<Territory>,
    pub routes: Vec<Route>,
    pub armies: Vec<Army>,
}

impl WorldSnapshot {
    #[must_use]
    pub fn person(&self, id: PersonId) -> Option<&Person> {
        self.people.iter().find(|person| person.id == id)
    }

    #[must_use]
    pub fn government(&self, id: GovernmentId) -> Option<&Government> {
        self.governments
            .iter()
            .find(|government| government.id == id)
    }

    #[must_use]
    pub fn territory(&self, id: TerritoryId) -> Option<&Territory> {
        self.territories.iter().find(|territory| territory.id == id)
    }

    #[must_use]
    pub fn route(&self, id: RouteId) -> Option<&Route> {
        self.routes.iter().find(|route| route.id == id)
    }

    #[must_use]
    pub fn army(&self, id: ArmyId) -> Option<&Army> {
        self.armies.iter().find(|army| army.id == id)
    }

    #[must_use]
    pub fn route_between(&self, from: TerritoryId, to: TerritoryId) -> Option<&Route> {
        self.routes.iter().find(|route| route.connects(from, to))
    }

    pub fn adjacent_territories(
        &self,
        territory: TerritoryId,
    ) -> impl Iterator<Item = TerritoryId> + '_ {
        self.routes
            .iter()
            .filter_map(move |route| route.other_end(territory))
    }

    #[must_use]
    pub fn travel_duration(&self, from: TerritoryId, to: TerritoryId) -> Option<SimDuration> {
        self.route_between(from, to)
            .map(|route| SimDuration::minutes(route.travel_minutes))
    }

    #[must_use]
    pub fn distance(&self, from: TerritoryId, to: TerritoryId) -> Option<f32> {
        let first = self.territory(from)?.position;
        let second = self.territory(to)?.position;
        Some((second.x - first.x).hypot(second.y - first.y))
    }

    pub fn territories_within(
        &self,
        center: TerritoryId,
        radius: f32,
    ) -> impl Iterator<Item = &Territory> {
        let center_position = self.territory(center).map(|territory| territory.position);
        self.territories.iter().filter(move |territory| {
            center_position.is_some_and(|point| {
                (territory.position.x - point.x).hypot(territory.position.y - point.y) <= radius
            })
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorldDiff {
    pub changed_armies: Vec<ArmyId>,
    pub changed_people: Vec<PersonId>,
    pub changed_territories: Vec<TerritoryId>,
}

impl WorldDiff {
    #[must_use]
    pub fn between(before: &WorldSnapshot, after: &WorldSnapshot) -> Self {
        let changed_armies = after
            .armies
            .iter()
            .filter(|army| before.army(army.id) != Some(*army))
            .map(|army| army.id)
            .collect();
        let changed_people = after
            .people
            .iter()
            .filter(|person| before.person(person.id) != Some(*person))
            .map(|person| person.id)
            .collect();
        let changed_territories = after
            .territories
            .iter()
            .filter(|territory| before.territory(territory.id) != Some(*territory))
            .map(|territory| territory.id)
            .collect();
        Self {
            changed_armies,
            changed_people,
            changed_territories,
        }
    }
}
