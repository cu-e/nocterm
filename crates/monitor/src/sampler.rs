//! Turns successive readings into loads, rates and history.

use crate::{
    CpuTimes, DiskCounters, Filesystem, History, HostInfo, NetworkCounters, Reading, Temperature,
    reading::percent,
};

/// What a host looks like now. A field is `None` when the latest reading
/// did not include it, or when a load or rate needs one more reading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub host: Option<HostInfo>,
    /// Total processor load, in percent.
    pub cpu: Option<f32>,
    /// Load of every core, in percent.
    pub cores: Option<Vec<f32>>,
    pub memory: Option<Usage>,
    pub swap: Option<Usage>,
    pub load: Option<[f64; 3]>,
    /// Seconds since the host started.
    pub uptime: Option<f64>,
    pub filesystems: Option<Vec<Filesystem>>,
    pub network: Option<NetworkRates>,
    pub disk_io: Option<DiskRates>,
    pub temperatures: Option<Vec<Temperature>>,
}

impl Snapshot {
    /// The warmest sensor.
    pub fn hottest(&self) -> Option<&Temperature> {
        self.temperatures
            .as_ref()?
            .iter()
            .max_by(|left, right| left.celsius.total_cmp(&right.celsius))
    }

    /// The file system closest to full.
    pub fn fullest(&self) -> Option<&Filesystem> {
        self.filesystems
            .as_ref()?
            .iter()
            .max_by(|left, right| left.percent().total_cmp(&right.percent()))
    }
}

/// Space or memory in use, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

impl Usage {
    pub fn percent(self) -> f32 {
        percent(self.used, self.total)
    }
}

/// Network throughput, in bytes per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetworkRates {
    pub received: f64,
    pub sent: f64,
}

/// Disk throughput, in bytes per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DiskRates {
    pub read: f64,
    pub written: f64,
}

/// Keeps the previous reading's counters to derive loads and rates.
///
/// One sampler serves one host across watches: counters are cumulative and
/// stamped with the host's clock, so a restarted script continues where the
/// last one stopped.
#[derive(Clone, Debug, Default)]
pub struct Sampler {
    snapshot: Snapshot,
    history: History,
    cpu: Option<CpuTimes>,
    cores: Option<Vec<CpuTimes>>,
    network: Option<(f64, NetworkCounters)>,
    disk_io: Option<(f64, DiskCounters)>,
}

impl Sampler {
    pub fn new(history_points: usize) -> Self {
        Self {
            history: History::new(history_points),
            ..Self::default()
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn set_history_points(&mut self, points: usize) {
        self.history.set_capacity(points);
    }

    /// Folds in the next reading. What it lacks is cleared, along with the
    /// counters its rates would have been measured from.
    pub fn apply(&mut self, reading: Reading) {
        let host = reading.host.or_else(|| self.snapshot.host.take());
        let clock = reading.clock;
        let mut snapshot = Snapshot {
            host,
            load: reading.load,
            uptime: reading.uptime,
            filesystems: reading.filesystems,
            temperatures: reading.temperatures,
            memory: reading.memory.map(|(total, available)| Usage {
                used: total.saturating_sub(available),
                total,
            }),
            swap: reading.swap.map(|(total, free)| Usage {
                used: total.saturating_sub(free),
                total,
            }),
            ..Snapshot::default()
        };

        snapshot.cpu = reading
            .cpu
            .zip(self.cpu)
            .and_then(|(now, before)| now.load_since(before));
        self.cpu = reading.cpu;
        snapshot.cores = match (&reading.cores, &self.cores) {
            (Some(now), Some(before)) if now.len() == before.len() => Some(
                now.iter()
                    .zip(before)
                    .map(|(now, before)| now.load_since(*before).unwrap_or_default())
                    .collect(),
            ),
            _ => None,
        };
        self.cores = reading.cores;

        snapshot.network = rate(&mut self.network, clock, reading.network, |now, before| {
            Some([
                now.received.checked_sub(before.received)?,
                now.sent.checked_sub(before.sent)?,
            ])
        })
        .map(|[received, sent]| NetworkRates { received, sent });
        snapshot.disk_io = rate(&mut self.disk_io, clock, reading.disk_io, |now, before| {
            Some([
                now.read.checked_sub(before.read)?,
                now.written.checked_sub(before.written)?,
            ])
        })
        .map(|[read, written]| DiskRates { read, written });

        if let Some(time) = clock {
            let history = &mut self.history;
            if let Some(cpu) = snapshot.cpu {
                history.cpu.push(time, cpu);
            }
            if let Some(memory) = snapshot.memory {
                history.memory.push(time, memory.percent());
            }
            if let Some(network) = snapshot.network {
                history.received.push(time, network.received as f32);
                history.sent.push(time, network.sent as f32);
            }
            if let Some(disk) = snapshot.disk_io {
                history.read.push(time, disk.read as f32);
                history.written.push(time, disk.written as f32);
            }
        }
        self.snapshot = snapshot;
    }
}

/// Per-second rates of two counters since the previous reading. A counter
/// that went down (an interface vanished, the host rebooted) restarts the
/// measurement.
fn rate<T: Copy>(
    previous: &mut Option<(f64, T)>,
    clock: Option<f64>,
    now: Option<T>,
    delta: impl Fn(T, T) -> Option<[u64; 2]>,
) -> Option<[f64; 2]> {
    let before = previous.take();
    let (clock, now) = (clock?, now?);
    *previous = Some((clock, now));
    let (then, before) = before?;
    let elapsed = clock - then;
    if elapsed <= 0.0 {
        return None;
    }
    let [first, second] = delta(now, before)?;
    Some([first as f64 / elapsed, second as f64 / elapsed])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(clock: f64, busy: u64, total: u64, received: u64) -> Reading {
        Reading {
            clock: Some(clock),
            cpu: Some(CpuTimes { busy, total }),
            cores: Some(vec![CpuTimes { busy, total }; 2]),
            memory: Some((1000, 250)),
            network: Some(NetworkCounters { received, sent: 0 }),
            ..Reading::default()
        }
    }

    #[test]
    fn loads_and_rates_need_two_readings() {
        let mut sampler = Sampler::new(10);
        sampler.apply(Reading {
            host: Some(HostInfo {
                hostname: "box".into(),
                ..HostInfo::default()
            }),
            ..reading(10.0, 0, 0, 1000)
        });
        let first = sampler.snapshot();
        assert_eq!(first.cpu, None);
        assert_eq!(first.network, None);
        assert_eq!(
            first.memory,
            Some(Usage {
                used: 750,
                total: 1000
            })
        );

        sampler.apply(reading(12.0, 50, 100, 5000));
        let second = sampler.snapshot();
        assert_eq!(second.cpu, Some(50.0));
        assert_eq!(second.cores, Some(vec![50.0, 50.0]));
        assert_eq!(
            second.network,
            Some(NetworkRates {
                received: 2000.0,
                sent: 0.0
            })
        );
        assert_eq!(
            second.host.as_ref().unwrap().hostname,
            "box",
            "host details persist"
        );
        assert_eq!(sampler.history().cpu.len(), 1);
        assert_eq!(sampler.history().memory.len(), 2);
    }

    #[test]
    fn a_missing_metric_clears_its_value_and_baseline() {
        let mut sampler = Sampler::new(10);
        sampler.apply(reading(10.0, 0, 0, 1000));
        sampler.apply(Reading {
            network: None,
            ..reading(11.0, 10, 100, 0)
        });
        assert_eq!(sampler.snapshot().network, None);
        sampler.apply(reading(12.0, 20, 200, 9000));
        assert_eq!(sampler.snapshot().network, None, "a new baseline first");
        sampler.apply(reading(13.0, 30, 300, 9500));
        assert_eq!(sampler.snapshot().network.unwrap().received, 500.0);
    }

    #[test]
    fn counters_that_go_back_restart_the_rate() {
        let mut sampler = Sampler::new(10);
        sampler.apply(reading(10.0, 0, 0, 9000));
        sampler.apply(reading(11.0, 0, 0, 100));
        assert_eq!(sampler.snapshot().network, None);
        sampler.apply(reading(12.0, 0, 0, 300));
        assert_eq!(sampler.snapshot().network.unwrap().received, 200.0);
    }
}
