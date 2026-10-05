//! Raw counters as one frame of a host's script reports them.

/// Everything one frame reported. A field is `None` when it was not asked
/// for or the host could not tell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    /// Seconds on the host's monotonic clock when the frame was taken; the
    /// time base for rates.
    pub clock: Option<f64>,
    pub host: Option<HostInfo>,
    pub cpu: Option<CpuTimes>,
    /// One entry per core, in the host's order.
    pub cores: Option<Vec<CpuTimes>>,
    /// Memory as (total, available) bytes.
    pub memory: Option<(u64, u64)>,
    /// Swap or page file as (total, free) bytes.
    pub swap: Option<(u64, u64)>,
    pub load: Option<[f64; 3]>,
    /// Seconds since the host started.
    pub uptime: Option<f64>,
    pub filesystems: Option<Vec<Filesystem>>,
    pub network: Option<NetworkCounters>,
    pub disk_io: Option<DiskCounters>,
    pub temperatures: Option<Vec<Temperature>>,
}

/// What a host says about itself once per watch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub cpu_model: String,
}

impl HostInfo {
    pub fn is_empty(&self) -> bool {
        self.hostname.is_empty()
            && self.os.is_empty()
            && self.kernel.is_empty()
            && self.cpu_model.is_empty()
    }
}

/// Cumulative processor time, in the host's own ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

impl CpuTimes {
    /// Percent busy between `earlier` and `self`, when time has passed.
    pub fn load_since(self, earlier: Self) -> Option<f32> {
        let total = self.total.checked_sub(earlier.total)?;
        let busy = self.busy.checked_sub(earlier.busy)?;
        (total > 0).then(|| (busy.min(total) as f64 / total as f64 * 100.0) as f32)
    }
}

/// One mounted file system.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filesystem {
    /// Where it is mounted: a path, or a drive such as `C:`.
    pub mount: String,
    pub total: u64,
    pub used: u64,
}

impl Filesystem {
    pub fn percent(&self) -> f32 {
        percent(self.used, self.total)
    }
}

/// Cumulative bytes over every physical network interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetworkCounters {
    pub received: u64,
    pub sent: u64,
}

/// Cumulative bytes over every physical disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskCounters {
    pub read: u64,
    pub written: u64,
}

/// One temperature sensor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Temperature {
    pub label: String,
    pub celsius: f32,
}

pub(crate) fn percent(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        (part.min(whole) as f64 / whole as f64 * 100.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_is_the_busy_share_of_elapsed_time() {
        let earlier = CpuTimes {
            busy: 100,
            total: 1000,
        };
        let later = CpuTimes {
            busy: 150,
            total: 1200,
        };
        assert_eq!(later.load_since(earlier), Some(25.0));
        assert_eq!(earlier.load_since(earlier), None);
        assert_eq!(earlier.load_since(later), None);
    }
}
