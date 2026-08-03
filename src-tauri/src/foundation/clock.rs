use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Serializable wall-clock time in milliseconds since the Unix epoch.
///
/// Times before the epoch clamp to zero and values beyond i64 clamp to
/// `i64::MAX`; callers therefore never receive wrapping timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(transparent)]
pub struct WallClockMillis(i64);

impl WallClockMillis {
    pub const EPOCH: Self = Self(0);

    pub const fn from_millis_saturating(value: u128) -> Self {
        if value > i64::MAX as u128 {
            Self(i64::MAX)
        } else {
            Self(value as i64)
        }
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Injectable clock boundary for code that needs deterministic time behavior.
pub trait Clock {
    fn wall_time(&self) -> WallClockMillis;
    fn monotonic_now(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn wall_time(&self) -> WallClockMillis {
        wall_time_from(SystemTime::now())
    }

    fn monotonic_now(&self) -> Instant {
        Instant::now()
    }
}

pub fn wall_time_from(value: SystemTime) -> WallClockMillis {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| WallClockMillis::from_millis_saturating(duration.as_millis()))
        .unwrap_or(WallClockMillis::EPOCH)
}

pub fn unix_time_ms_i64() -> i64 {
    SystemClock.wall_time().get()
}

pub fn unix_time_ms_u64() -> u64 {
    unix_time_ms_u128().min(u64::MAX as u128) as u64
}

pub fn unix_time_ms_u128() -> u128 {
    unix_time_ms_from(SystemTime::now()).unwrap_or_default()
}

pub fn unix_time_ms_from(value: SystemTime) -> Option<u128> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
}

pub fn unix_time_secs_i64() -> i64 {
    unix_time_secs_u64().min(i64::MAX as u64) as i64
}

pub fn unix_time_secs_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub fn unix_time_ns_u128() -> u128 {
    unix_time_ns_from(SystemTime::now()).unwrap_or_default()
}

pub fn unix_time_ns_from(value: SystemTime) -> Option<u128> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

/// Monotonic elapsed time is deliberately returned as a duration and is never
/// serialized as wall-clock evidence.
pub fn monotonic_elapsed(started_at: Instant) -> Duration {
    started_at.elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_clamps_pre_epoch_and_overflow() {
        assert_eq!(
            wall_time_from(UNIX_EPOCH - Duration::from_secs(1)),
            WallClockMillis::EPOCH
        );
        assert_eq!(
            WallClockMillis::from_millis_saturating(u128::MAX).get(),
            i64::MAX
        );
    }

    #[test]
    fn wall_clock_serializes_as_integer_milliseconds() {
        assert_eq!(
            serde_json::to_string(&WallClockMillis::from_millis_saturating(42)).unwrap(),
            "42"
        );
    }

    #[test]
    fn explicit_units_preserve_epoch_offsets() {
        let value = UNIX_EPOCH + Duration::new(42, 123_456_789);
        assert_eq!(unix_time_ms_from(value), Some(42_123));
        assert_eq!(unix_time_ns_from(value), Some(42_123_456_789));
    }

    #[test]
    fn explicit_unit_conversions_reject_pre_epoch_values() {
        let value = UNIX_EPOCH - Duration::from_nanos(1);
        assert_eq!(unix_time_ms_from(value), None);
        assert_eq!(unix_time_ns_from(value), None);
    }

    #[test]
    fn monotonic_elapsed_never_uses_wall_clock() {
        let started_at = Instant::now();
        assert!(monotonic_elapsed(started_at) <= Duration::from_secs(1));
    }
}
