//! Historical simulation time, independent from rendering frames and wall time.

use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::ops::{Add, AddAssign, Sub};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SimDuration(i64);

impl SimDuration {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn minutes(minutes: i64) -> Self {
        Self(minutes)
    }

    #[must_use]
    pub const fn hours(hours: i64) -> Self {
        Self(hours.saturating_mul(60))
    }

    #[must_use]
    pub const fn days(days: i64) -> Self {
        Self(days.saturating_mul(24 * 60))
    }

    #[must_use]
    pub const fn as_minutes(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SimTime(i64);

impl SimTime {
    pub const EPOCH: Self = Self(0);

    #[must_use]
    pub const fn from_minutes(minutes_from_epoch: i64) -> Self {
        Self(minutes_from_epoch)
    }

    #[must_use]
    pub const fn as_minutes(self) -> i64 {
        self.0
    }
}

impl Add<SimDuration> for SimTime {
    type Output = Self;

    fn add(self, duration: SimDuration) -> Self::Output {
        Self(self.0.saturating_add(duration.0))
    }
}

impl Add for SimDuration {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        Self(self.0.saturating_add(other.0))
    }
}

impl AddAssign<SimDuration> for SimTime {
    fn add_assign(&mut self, duration: SimDuration) {
        *self = *self + duration;
    }
}

impl Sub for SimTime {
    type Output = SimDuration;

    fn sub(self, other: Self) -> Self::Output {
        SimDuration(self.0.saturating_sub(other.0))
    }
}

impl Display for SimTime {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let day = self.0.div_euclid(24 * 60);
        let within_day = self.0.rem_euclid(24 * 60);
        let hour = within_day / 60;
        let minute = within_day % 60;
        write!(formatter, "Day {day}, {hour:02}:{minute:02}")
    }
}
