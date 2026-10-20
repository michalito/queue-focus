use std::io;

/// Wall seconds for legacy rendering plus boot nanoseconds including suspend.
/// Boot observations are process-local and are never persisted across reboot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSample {
    pub wall: u64,
    pub boot: u64,
}
impl ClockSample {
    pub fn now() -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        let id = libc::CLOCK_MONOTONIC_RAW;
        #[cfg(not(target_os = "macos"))]
        let id = libc::CLOCK_BOOTTIME;
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: value is a valid writable timespec for the duration of call.
        if unsafe { libc::clock_gettime(id, &mut value) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let seconds =
            u64::try_from(value.tv_sec).map_err(|_| io::Error::other("invalid boot clock"))?;
        let nanos =
            u64::try_from(value.tv_nsec).map_err(|_| io::Error::other("invalid boot clock"))?;
        Ok(Self {
            wall: crate::unix_now(),
            boot: seconds.saturating_mul(1_000_000_000).saturating_add(nanos),
        })
    }
    pub fn seconds_since(self, previous: Self) -> Option<u64> {
        self.boot
            .checked_sub(previous.boot)
            .map(|n| n / 1_000_000_000)
    }
}
