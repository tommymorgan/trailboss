//! Same-host monotonic timestamps for private live-runtime snapshots.
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

fn monotonic_nanos() -> Result<i128, String> {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, time.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let time = unsafe { time.assume_init() };
    Ok(i128::from(time.tv_sec) * 1_000_000_000 + i128::from(time.tv_nsec))
}
fn encode(value: Instant) -> Result<i128, String> {
    let now = Instant::now();
    let monotonic = monotonic_nanos()?;
    Ok(if value >= now {
        monotonic + value.duration_since(now).as_nanos() as i128
    } else {
        monotonic - now.duration_since(value).as_nanos() as i128
    })
}
fn decode(value: i128) -> Result<Instant, String> {
    let monotonic = monotonic_nanos()?;
    let now = Instant::now();
    let delta = (value - monotonic).unsigned_abs();
    let seconds = u64::try_from(delta / 1_000_000_000)
        .map_err(|_| "snapshot monotonic timestamp overflow")?;
    let delta = Duration::new(seconds, (delta % 1_000_000_000) as u32);
    if value >= monotonic {
        now.checked_add(delta)
    } else {
        now.checked_sub(delta)
    }
    .ok_or_else(|| "snapshot instant outside platform range".into())
}
pub(crate) mod instant {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &Instant,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        encode(*value)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Instant, D::Error> {
        decode(i128::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
pub(crate) mod optional {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &Option<Instant>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .map(encode)
            .transpose()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Instant>, D::Error> {
        Option::<i128>::deserialize(deserializer)?
            .map(decode)
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
pub(crate) mod map {
    use super::*;
    use std::collections::HashMap;
    pub fn serialize<S: serde::Serializer>(
        value: &HashMap<String, Instant>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .iter()
            .map(|(key, value)| encode(*value).map(|stamp| (key, stamp)))
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<HashMap<String, Instant>, D::Error> {
        HashMap::<String, i128>::deserialize(deserializer)?
            .into_iter()
            .map(|(key, value)| decode(value).map(|instant| (key, instant)))
            .collect::<Result<_, _>>()
            .map_err(serde::de::Error::custom)
    }
}
