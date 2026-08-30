//! Reference-world projection retained for the current runtime facade.
//!
//! This type is not a persistence migration path; Format 8 rejects old saves.

use canwu_core::{
    ArmyId, EntityRef, GovernmentId, LetterId, PersonId, ResourceId, RouteId, TerritoryId,
};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transit: Option<PersonTransitState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersonTransitState {
    pub from: TerritoryId,
    pub to: TerritoryId,
    pub departed_at: SimTime,
    pub arrives_at: SimTime,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LetterStatus {
    HeldByPerson,
    InTransit,
    HeldAtLocation,
    Delivered,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LetterCargo {
    pub id: LetterId,
    pub sender: PersonId,
    pub recipient: PersonId,
    pub body: String,
    pub status: LetterStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier: Option<PersonId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<TerritoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<SimTime>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub letters: Vec<LetterCargo>,
}

impl WorldSnapshot {
    #[must_use]
    pub fn entities(&self) -> Vec<EntityRef> {
        let mut entities = Vec::with_capacity(
            self.armies.len()
                + self.governments.len()
                + self.people.len()
                + self.letters.len()
                + self.routes.len()
                + self.territories.len(),
        );
        entities.extend(self.armies.iter().map(|value| EntityRef::Army(value.id)));
        entities.extend(
            self.governments
                .iter()
                .map(|value| EntityRef::Government(value.id)),
        );
        entities.extend(self.people.iter().map(|value| EntityRef::Person(value.id)));
        entities.extend(
            self.letters
                .iter()
                .map(|value| EntityRef::Resource(ResourceId::new(value.id.get()))),
        );
        entities.extend(self.routes.iter().map(|value| EntityRef::Route(value.id)));
        entities.extend(
            self.territories
                .iter()
                .map(|value| EntityRef::Territory(value.id)),
        );
        entities.sort();
        entities
    }

    #[must_use]
    pub fn person(&self, id: PersonId) -> Option<&Person> {
        self.people.iter().find(|person| person.id == id)
    }

    #[must_use]
    pub fn government(&self, id: GovernmentId) -> Option<&Government> {
        self.governments.iter().find(|value| value.id == id)
    }

    #[must_use]
    pub fn territory(&self, id: TerritoryId) -> Option<&Territory> {
        self.territories.iter().find(|value| value.id == id)
    }

    #[must_use]
    pub fn route(&self, id: RouteId) -> Option<&Route> {
        self.routes.iter().find(|value| value.id == id)
    }

    #[must_use]
    pub fn army(&self, id: ArmyId) -> Option<&Army> {
        self.armies.iter().find(|value| value.id == id)
    }

    #[must_use]
    pub fn letter(&self, id: LetterId) -> Option<&LetterCargo> {
        self.letters.iter().find(|value| value.id == id)
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
