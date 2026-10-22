use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerEpoch {
    pub last_sleep: u64,
    pub last_wake: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeBoundary {
    pub earliest_millis: i64,
    pub latest_millis: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerEpochSnapshot {
    pub epoch: PowerEpoch,
    pub wake_boundary: Option<WakeBoundary>,
    pub clock_anchor: Option<PowerClockAnchor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerClockAnchor {
    pub interrupt_before: u64,
    pub interrupt_after: u64,
    pub wall_unix_100ns: u64,
}

impl PowerClockAnchor {
    /// # Errors
    /// 唤醒计数早于已确认的时钟锚或算术越界时不推算截止时间。
    pub fn subsequent_wake_boundary(self, wake: u64) -> io::Result<WakeBoundary> {
        if self.interrupt_before > self.interrupt_after || wake < self.interrupt_after {
            return Err(unavailable());
        }
        let millis = |origin: u64| {
            self.wall_unix_100ns
                .checked_add(wake - origin)
                .and_then(|ticks| i64::try_from(ticks / 10_000).ok())
                .ok_or_else(unavailable)
        };
        Ok(WakeBoundary {
            earliest_millis: millis(self.interrupt_after)?,
            latest_millis: millis(self.interrupt_before)?,
        })
    }
}

/// # Errors
/// 内核电源记录不可读、发生过渡或不能确认完整的一对记录时拒绝自动调度。
pub fn snapshot() -> io::Result<PowerEpochSnapshot> {
    native::snapshot()
}

fn unavailable() -> io::Error {
    io::Error::other("automation_power_epoch_unconfirmed")
}

fn validate_pair(epoch: PowerEpoch) -> io::Result<()> {
    if (epoch.last_sleep == 0 && epoch.last_wake != 0)
        || (epoch.last_sleep != 0 && epoch.last_wake <= epoch.last_sleep)
    {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod native {
    use super::{PowerEpoch, PowerEpochSnapshot, WakeBoundary, io, unavailable, validate_pair};
    use std::ffi::CStr;

    #[allow(unsafe_code)]
    fn read<T: Default>(name: &CStr) -> io::Result<T> {
        let mut value = T::default();
        let mut size = size_of::<T>();
        let result = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut value).cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 || size != size_of::<T>() {
            return Err(unavailable());
        }
        Ok(value)
    }

    fn epoch() -> io::Result<PowerEpoch> {
        Ok(PowerEpoch {
            last_sleep: read(c"kern.sleep_abs_time")?,
            last_wake: read(c"kern.wake_abs_time")?,
        })
    }

    pub(super) fn snapshot() -> io::Result<PowerEpochSnapshot> {
        let before = epoch()?;
        let wake: libc::timeval = read(c"kern.waketime")?;
        let after = epoch()?;
        if before != after || !(0..1_000_000).contains(&wake.tv_usec) {
            return Err(unavailable());
        }
        validate_pair(after)?;
        let wake_boundary = if after.last_wake == 0 {
            if wake.tv_sec != 0 || wake.tv_usec != 0 {
                return Err(unavailable());
            }
            None
        } else {
            let millis = wake
                .tv_sec
                .checked_mul(1000)
                .and_then(|seconds| seconds.checked_add(i64::from(wake.tv_usec) / 1000))
                .filter(|value| *value > 0)
                .ok_or_else(unavailable)?;
            Some(WakeBoundary {
                earliest_millis: millis,
                latest_millis: millis,
            })
        };
        Ok(PowerEpochSnapshot {
            epoch: after,
            wake_boundary,
            clock_anchor: None,
        })
    }
}

#[cfg(target_os = "windows")]
mod native {
    use super::{
        PowerClockAnchor, PowerEpoch, PowerEpochSnapshot, WakeBoundary, io, unavailable,
        validate_pair,
    };
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            Power::{CallNtPowerInformation, LastSleepTime, LastWakeTime, POWER_INFORMATION_LEVEL},
            SystemInformation::GetSystemTimePreciseAsFileTime,
            WindowsProgramming::QueryInterruptTimePrecise,
        },
    };

    #[allow(unsafe_code)]
    fn read(level: POWER_INFORMATION_LEVEL) -> io::Result<u64> {
        let mut value = 0_u64;
        let result = unsafe {
            CallNtPowerInformation(level, std::ptr::null(), 0, (&raw mut value).cast(), 8)
        };
        if result != 0 {
            return Err(unavailable());
        }
        Ok(value)
    }

    fn epoch() -> io::Result<PowerEpoch> {
        Ok(PowerEpoch {
            last_sleep: read(LastSleepTime)?,
            last_wake: read(LastWakeTime)?,
        })
    }

    fn unix_millis(file_time: u64) -> io::Result<i64> {
        i64::try_from(
            file_time
                .checked_sub(116_444_736_000_000_000)
                .ok_or_else(unavailable)?
                / 10_000,
        )
        .map_err(|_| unavailable())
    }

    #[allow(unsafe_code)]
    pub(super) fn snapshot() -> io::Result<PowerEpochSnapshot> {
        let before = epoch()?;
        let mut first = 0;
        let mut last = 0;
        let mut wall = FILETIME::default();
        unsafe {
            QueryInterruptTimePrecise(&raw mut first);
            GetSystemTimePreciseAsFileTime(&raw mut wall);
            QueryInterruptTimePrecise(&raw mut last);
        }
        let after = epoch()?;
        if before != after || first > last || after.last_wake > first {
            return Err(unavailable());
        }
        validate_pair(after)?;
        let wall = (u64::from(wall.dwHighDateTime) << 32) | u64::from(wall.dwLowDateTime);
        let wake_boundary = if after.last_wake == 0 {
            None
        } else {
            Some(WakeBoundary {
                earliest_millis: unix_millis(
                    wall.checked_sub(last - after.last_wake)
                        .ok_or_else(unavailable)?,
                )?,
                latest_millis: unix_millis(
                    wall.checked_sub(first - after.last_wake)
                        .ok_or_else(unavailable)?,
                )?,
            })
        };
        Ok(PowerEpochSnapshot {
            epoch: after,
            wake_boundary,
            clock_anchor: Some(PowerClockAnchor {
                interrupt_before: first,
                interrupt_after: last,
                wall_unix_100ns: wall
                    .checked_sub(116_444_736_000_000_000)
                    .ok_or_else(unavailable)?,
            }),
        })
    }
}

#[cfg(target_os = "linux")]
mod native {
    use super::{PowerEpochSnapshot, io};
    pub(super) fn snapshot() -> io::Result<PowerEpochSnapshot> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "automation_power_epoch_not_supported",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PowerClockAnchor, PowerEpoch, validate_pair};

    #[test]
    fn never_slept_is_distinct_from_an_incomplete_power_transition() {
        assert!(
            validate_pair(PowerEpoch {
                last_sleep: 0,
                last_wake: 0
            })
            .is_ok()
        );
        assert!(
            validate_pair(PowerEpoch {
                last_sleep: 3,
                last_wake: 8
            })
            .is_ok()
        );
        for epoch in [
            PowerEpoch {
                last_sleep: 0,
                last_wake: 2,
            },
            PowerEpoch {
                last_sleep: 5,
                last_wake: 5,
            },
            PowerEpoch {
                last_sleep: 9,
                last_wake: 5,
            },
        ] {
            assert!(validate_pair(epoch).is_err());
        }
    }

    #[test]
    fn a_previous_clock_anchor_bounds_the_next_wake_without_an_elapsed_time_threshold() {
        let anchor = PowerClockAnchor {
            interrupt_before: 10_000,
            interrupt_after: 10_009,
            wall_unix_100ns: 1_000_000,
        };
        let boundary = anchor.subsequent_wake_boundary(30_000).unwrap();
        assert_eq!(boundary.earliest_millis, 101);
        assert_eq!(boundary.latest_millis, 102);
        assert!(anchor.subsequent_wake_boundary(10_008).is_err());
    }

    #[test]
    #[ignore = "由隔离宿主显式读取真实系统记录，不改变系统电源状态"]
    fn native_power_epoch_is_read_only_and_has_a_stable_kernel_pair() {
        let sample = super::snapshot().unwrap();
        validate_pair(sample.epoch).unwrap();
        eprintln!(
            "native_power_epoch={}",
            serde_json::to_string(&sample).unwrap()
        );
    }
}
