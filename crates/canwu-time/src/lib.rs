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

    /// Creates a duration from hours.
    ///
    /// # Panics
    ///
    /// Panics when the value cannot be represented in simulation minutes. Use
    /// [`Self::checked_hours`] for data-dependent input.
    #[must_use]
    pub const fn hours(hours: i64) -> Self {
        match Self::checked_hours(hours) {
            Some(duration) => duration,
            None => panic!("simulation duration hours exceed the supported range"),
        }
    }

    #[must_use]
    pub const fn checked_hours(hours: i64) -> Option<Self> {
        match hours.checked_mul(60) {
            Some(minutes) => Some(Self(minutes)),
            None => None,
        }
    }

    /// Creates a duration from days.
    ///
    /// # Panics
    ///
    /// Panics when the value cannot be represented in simulation minutes. Use
    /// [`Self::checked_days`] for data-dependent input.
    #[must_use]
    pub const fn days(days: i64) -> Self {
        match Self::checked_days(days) {
            Some(duration) => duration,
            None => panic!("simulation duration days exceed the supported range"),
        }
    }

    #[must_use]
    pub const fn checked_days(days: i64) -> Option<Self> {
        match days.checked_mul(24 * 60) {
            Some(minutes) => Some(Self(minutes)),
            None => None,
        }
    }

    #[must_use]
    pub const fn as_minutes(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(minutes) => Some(Self(minutes)),
            None => None,
        }
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

    #[must_use]
    pub const fn checked_add(self, duration: SimDuration) -> Option<Self> {
        match self.0.checked_add(duration.0) {
            Some(minutes) => Some(Self(minutes)),
            None => None,
        }
    }

    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<SimDuration> {
        match self.0.checked_sub(other.0) {
            Some(minutes) => Some(SimDuration(minutes)),
            None => None,
        }
    }
}

impl Add<SimDuration> for SimTime {
    type Output = Self;

    fn add(self, duration: SimDuration) -> Self::Output {
        self.checked_add(duration)
            .expect("simulation time addition exceeds the supported range")
    }
}

impl Add for SimDuration {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        self.checked_add(other)
            .expect("simulation duration addition exceeds the supported range")
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
        self.checked_sub(other)
            .expect("simulation time subtraction exceeds the supported range")
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

#[cfg(test)]
mod tests {
    use super::{SimDuration, SimTime};

    #[test]
    fn checked_time_apis_reject_overflow_without_silent_saturation() {
        assert_eq!(SimDuration::checked_hours(i64::MAX), None);
        assert_eq!(SimDuration::checked_days(i64::MAX), None);
        assert_eq!(
            SimDuration::minutes(i64::MAX).checked_add(SimDuration::minutes(1)),
            None
        );
        assert_eq!(
            SimTime::from_minutes(i64::MAX).checked_add(SimDuration::minutes(1)),
            None
        );
        assert_eq!(
            SimTime::from_minutes(i64::MAX).checked_sub(SimTime::from_minutes(i64::MIN)),
            None
        );

        assert!(
            std::panic::catch_unwind(|| SimDuration::hours(i64::MAX)).is_err(),
            "the convenience constructor must not clamp"
        );
        assert!(
            std::panic::catch_unwind(|| {
                SimTime::from_minutes(i64::MAX) + SimDuration::minutes(1)
            })
            .is_err(),
            "the addition operator must not clamp"
        );
    }
}
